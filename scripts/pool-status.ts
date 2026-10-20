/**
 * Pool health dashboard data endpoint.
 *
 * Polls the vault's read-only functions — TVL, reward rate, reward pool
 * balance, projected runway, and pause state — and prints a single JSON
 * document to stdout, suitable for a status page or an alerting
 * pipeline. Exits non-zero when a configured alert threshold is
 * breached, so this script doubles as a monitoring check on its own
 * (see "Exit codes" below).
 *
 * # Running it
 *
 *   CONTRACT_ID=C... IDENTITY=alice npx tsx scripts/pool-status.ts
 *
 *   # Mainnet has no default RPC endpoint in the Stellar CLI ("Bring
 *   # Your Own"), so point at your own provider explicitly:
 *   CONTRACT_ID=C... IDENTITY=ops npx tsx scripts/pool-status.ts \
 *     --network mainnet \
 *     --rpc-url https://your-mainnet-rpc-provider.example \
 *     --network-passphrase "Public Global Stellar Network ; September 2015"
 *
 * See the "Pool Health Dashboard Endpoint" section of the README for a
 * cron/CI usage example.
 *
 * # Exit codes
 *
 *   0 - healthy: fetched successfully, no alert threshold breached.
 *   1 - fetched successfully, but at least one alert threshold breached
 *       (e.g. low runway, or the pool is paused).
 *   2 - could not fetch pool state at all (RPC/CLI failure, bad
 *       arguments). Deliberately distinct from `1` so a monitoring
 *       pipeline can tell "the pool is in trouble" apart from
 *       "the monitoring itself is broken."
 *
 * (`tsx`/`ts-node` run this directly with no project `package.json`
 * required — this file has no dependency beyond Node's own
 * `node:child_process`.)
 */

import { execFileSync } from "node:child_process";

interface PoolStatus {
  timestamp: string;
  network: string;
  contractId: string;
  decimals: number;
  tvl: number;
  tvlRaw: string;
  rateBps: number;
  rewardPoolBalance: number;
  rewardPoolBalanceRaw: string;
  projectedRunwayLedgers: number;
  paused: boolean;
  alerts: string[];
  ok: boolean;
}

interface Args {
  contractId: string;
  identity: string;
  network: string;
  rpcUrl?: string;
  networkPassphrase?: string;
  decimals: number;
  minRunwayLedgers: number;
  alertOnPause: boolean;
  help: boolean;
}

function parseArgs(argv: string[]): Args {
  const args: Args = {
    contractId: process.env.CONTRACT_ID ?? "",
    identity: process.env.IDENTITY ?? "",
    network: process.env.NETWORK ?? "testnet",
    rpcUrl: process.env.RPC_URL ?? process.env.STELLAR_RPC_URL,
    networkPassphrase: process.env.NETWORK_PASSPHRASE ?? process.env.STELLAR_NETWORK_PASSPHRASE,
    decimals: process.env.DECIMALS ? Number(process.env.DECIMALS) : 7,
    minRunwayLedgers: process.env.MIN_RUNWAY_LEDGERS ? Number(process.env.MIN_RUNWAY_LEDGERS) : 17_280 * 7, // ~7 days at 5s/ledger
    alertOnPause: (process.env.ALERT_ON_PAUSE ?? "true") !== "false",
    help: false,
  };

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const next = (): string => {
      const value = argv[++i];
      if (value === undefined) {
        throw new Error(`Missing value for ${arg}`);
      }
      return value;
    };

    switch (arg) {
      case "--help":
      case "-h":
        args.help = true;
        break;
      case "--contract-id":
        args.contractId = next();
        break;
      case "--identity":
        args.identity = next();
        break;
      case "--network":
        args.network = next();
        break;
      case "--rpc-url":
        args.rpcUrl = next();
        break;
      case "--network-passphrase":
        args.networkPassphrase = next();
        break;
      case "--decimals":
        args.decimals = Number(next());
        break;
      case "--min-runway-ledgers":
        args.minRunwayLedgers = Number(next());
        break;
      case "--no-alert-on-pause":
        args.alertOnPause = false;
        break;
      default:
        console.error(`Unknown argument: ${arg}`);
        args.help = true;
    }
  }

  return args;
}

function printHelp(): void {
  console.log(`Pool health dashboard data endpoint

Usage:
  npx tsx scripts/pool-status.ts [options]

Target (required, via flag or environment variable):
  --contract-id <id>          Deployed vault contract id (or $CONTRACT_ID)
  --identity <name>           Local Stellar identity used as --source (or
                               $IDENTITY) — required by the CLI even for a
                               read-only simulated call
  --network <name>            Named Stellar network (or $NETWORK, default
                               testnet)
  --rpc-url <url>             RPC endpoint (or $RPC_URL / $STELLAR_RPC_URL).
                               Required for mainnet, which has no default
                               endpoint in the Stellar CLI ("Bring Your
                               Own") — overrides --network's named preset
                               when set
  --network-passphrase <p>    Network passphrase (or $NETWORK_PASSPHRASE /
                               $STELLAR_NETWORK_PASSPHRASE), used with
                               --rpc-url
  --decimals <n>              Token decimals for base-unit conversion (or
                               $DECIMALS, default 7)

Alert thresholds:
  --min-runway-ledgers <n>    Alert if the projected reward-pool runway
                               (ledgers) drops below this (or
                               $MIN_RUNWAY_LEDGERS, default 120960 = ~7
                               days at 5s/ledger)
  --no-alert-on-pause         Don't alert when the pool is paused (or
                               $ALERT_ON_PAUSE=false; alerting on pause is
                               on by default)

  -h, --help                  Show this help

Exit codes: 0 healthy, 1 an alert threshold was breached, 2 couldn't
fetch pool state at all. See this file's header comment for details.
`);
}

function invokeReadOnly(args: Args, fn: string): string {
  const cliArgs = ["contract", "invoke", "--id", args.contractId, "--source", args.identity];
  if (args.rpcUrl) {
    cliArgs.push("--rpc-url", args.rpcUrl);
    if (args.networkPassphrase) {
      cliArgs.push("--network-passphrase", args.networkPassphrase);
    }
  } else {
    cliArgs.push("--network", args.network);
  }
  cliArgs.push("--send=no", "--", fn);
  return execFileSync("stellar", cliArgs, { encoding: "utf8" }).trim();
}

/** Extracts the last digit run from the CLI's raw output as a string
 * (kept as a string, not `Number`, since TVL/reward-pool values are
 * i128s that can exceed `Number.MAX_SAFE_INTEGER`). Robust to the
 * handful of ways a single return value or a tuple of them might be
 * printed (`"25000000"`, `[25000000, 100000000]`, …). */
function parseLastNumber(raw: string): string {
  const matches = raw.match(/-?\d+/g);
  if (!matches || matches.length === 0) {
    throw new Error(`Could not parse numeric contract output: ${raw}`);
  }
  return matches[matches.length - 1]!;
}

function parseBool(raw: string): boolean {
  const cleaned = raw.trim().replace(/^"|"$/g, "").toLowerCase();
  if (cleaned === "true") return true;
  if (cleaned === "false") return false;
  throw new Error(`Could not parse boolean contract output: ${raw}`);
}

function fetchStatus(args: Args): PoolStatus {
  // vault_state() -> (total_shares, total_deposited); TVL is the second element.
  const tvlRaw = parseLastNumber(invokeReadOnly(args, "vault_state"));
  const rateBps = Number(parseLastNumber(invokeReadOnly(args, "get_reward_rate_bps")));
  const rewardPoolRaw = parseLastNumber(invokeReadOnly(args, "get_reward_pool_balance"));
  const projectedRunwayLedgers = Number(parseLastNumber(invokeReadOnly(args, "get_projected_runway")));
  const paused = parseBool(invokeReadOnly(args, "is_paused"));

  const scale = 10 ** args.decimals;
  const alerts: string[] = [];
  if (projectedRunwayLedgers < args.minRunwayLedgers) {
    alerts.push(
      `projected reward-pool runway (${projectedRunwayLedgers} ledgers) is below the ${args.minRunwayLedgers}-ledger threshold`,
    );
  }
  if (args.alertOnPause && paused) {
    alerts.push("pool is paused");
  }

  return {
    timestamp: new Date().toISOString(),
    network: args.rpcUrl ?? args.network,
    contractId: args.contractId,
    decimals: args.decimals,
    tvl: Number(tvlRaw) / scale,
    tvlRaw,
    rateBps,
    rewardPoolBalance: Number(rewardPoolRaw) / scale,
    rewardPoolBalanceRaw: rewardPoolRaw,
    projectedRunwayLedgers,
    paused,
    alerts,
    ok: alerts.length === 0,
  };
}

function main(): void {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    printHelp();
    return;
  }

  if (!args.contractId) {
    console.error("Error: --contract-id (or $CONTRACT_ID) is required.");
    printHelp();
    process.exitCode = 2;
    return;
  }
  if (!args.identity) {
    console.error("Error: --identity (or $IDENTITY) is required.");
    printHelp();
    process.exitCode = 2;
    return;
  }

  let status: PoolStatus;
  try {
    status = fetchStatus(args);
  } catch (err) {
    console.error(`Failed to fetch pool status: ${(err as Error).message}`);
    process.exitCode = 2;
    return;
  }

  console.log(JSON.stringify(status, null, 2));
  if (!status.ok) {
    process.exitCode = 1;
  }
}

main();
