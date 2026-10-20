/**
 * Reward pool runway simulator.
 *
 * Projects when the vault's reward pool would run dry under three TVL
 * growth scenarios (low / base / high), so an operator can size
 * `fund_reward_pool` ahead of time instead of guessing (see the "Reward
 * Pool Runway Simulation" section of the README for usage).
 *
 * # Shared math
 *
 * This uses the exact same pool-wide APR formula as
 * `runway_guard::projected_runway_ledgers` / `reward_math::reward_for_ledgers`
 * in the contract (with every boost multiplier at its neutral 1x, since
 * those are per-position, not pool-wide):
 *
 *   drain_over(ledgers) = tvl * rate_bps * ledgers / (BOOST_BPS_BASE * STELLAR_LEDGERS_PER_YEAR)
 *
 * `BOOST_BPS_BASE`, `STELLAR_LEDGERS_PER_YEAR`, and `LEDGERS_PER_DAY` below
 * are copied from the `pub(crate) const` declarations of the same names
 * near the top of `src/vault.rs`. If those ever change on-chain, update
 * the copies here too — that's the deliberate single point of drift this
 * script accepts in exchange for not depending on the Rust build at all.
 *
 * # Running it
 *
 *   npx tsx scripts/runway-sim.ts --tvl 1000000 --rate-bps 800 --reward-pool 15000
 *   npx tsx scripts/runway-sim.ts --contract-id C... --network testnet
 *   npx tsx scripts/runway-sim.ts --help
 *
 * (`tsx`/`ts-node` run this directly with no project `package.json`
 * required; see the README for alternatives.)
 */

import { execFileSync } from "node:child_process";

// ---------------------------------------------------------------------------
// Constants shared with the contract (src/vault.rs)
// ---------------------------------------------------------------------------

/** Mirrors `vault::BOOST_BPS_BASE` — the basis-point scale (100% = 10 000). */
const BOOST_BPS_BASE = 10_000;

/** Mirrors `vault::STELLAR_LEDGERS_PER_YEAR` (~5s per ledger). */
const STELLAR_LEDGERS_PER_YEAR = 6_307_200;

/** Mirrors `vault::LEDGERS_PER_DAY` (STELLAR_LEDGERS_PER_YEAR / 365). */
const LEDGERS_PER_DAY = 17_280;

const DAYS_PER_MONTH = 30; // simulation granularity only — not an on-chain constant

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface PoolState {
  /** Total deposited (TVL), in whole tokens (human-readable, not base units). */
  tvl: number;
  /** Current annualized reward rate, in basis points — matches `get_reward_rate_bps()`. */
  rateBps: number;
  /** Current reward pool balance, in whole tokens. */
  rewardPool: number;
}

interface ScenarioResult {
  name: string;
  annualGrowthRate: number;
  /** monthlyBalances[0] is month 0 (today); index i is month i. */
  monthlyBalances: number[];
  /** First month the balance hits zero, or `null` if it survives the horizon. */
  exhaustionMonth: number | null;
}

// ---------------------------------------------------------------------------
// Core math
// ---------------------------------------------------------------------------

/** Reward-pool spend over `days` days, at `rateBps` against `tvl` — the
 * same formula as `reward_math::reward_for_ledgers` with every multiplier
 * neutral, applied pool-wide (amount = tvl) rather than per position. */
function drainOverDays(tvl: number, rateBps: number, days: number): number {
  const ledgers = days * LEDGERS_PER_DAY;
  return (tvl * rateBps * ledgers) / (BOOST_BPS_BASE * STELLAR_LEDGERS_PER_YEAR);
}

/** TVL `months` months from now, compounding `annualGrowthRate` monthly. */
function projectedTvl(initialTvl: number, annualGrowthRate: number, months: number): number {
  const monthlyRate = Math.pow(1 + annualGrowthRate, 1 / 12) - 1;
  return initialTvl * Math.pow(1 + monthlyRate, months);
}

function simulateScenario(
  name: string,
  state: PoolState,
  annualGrowthRate: number,
  horizonMonths: number,
): ScenarioResult {
  let balance = state.rewardPool;
  const monthlyBalances: number[] = [balance];
  let exhaustionMonth: number | null = balance <= 0 ? 0 : null;

  for (let month = 1; month <= horizonMonths; month++) {
    // Average TVL across the month, since a growing/shrinking TVL changes
    // the drain rate continuously, not in a single jump at month-end.
    const tvlStart = projectedTvl(state.tvl, annualGrowthRate, month - 1);
    const tvlEnd = projectedTvl(state.tvl, annualGrowthRate, month);
    const avgTvl = (tvlStart + tvlEnd) / 2;

    const spend = drainOverDays(avgTvl, state.rateBps, DAYS_PER_MONTH);
    balance = Math.max(0, balance - spend);
    monthlyBalances.push(balance);
    if (exhaustionMonth === null && balance <= 0) {
      exhaustionMonth = month;
    }
  }

  return { name, annualGrowthRate, monthlyBalances, exhaustionMonth };
}

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

interface Args {
  tvl?: number;
  rateBps?: number;
  rewardPool?: number;
  fund: number;
  decimals: number;
  horizonMonths: number;
  lowGrowth: number;
  baseGrowth: number;
  highGrowth: number;
  contractId?: string;
  network: string;
  identity?: string;
  help: boolean;
}

function parseArgs(argv: string[]): Args {
  const args: Args = {
    fund: 0,
    decimals: 7, // Soroban/Stellar token convention; override with --decimals if your token differs
    horizonMonths: 24,
    lowGrowth: -0.25,
    baseGrowth: 0.5,
    highGrowth: 2.0,
    network: process.env.NETWORK ?? "testnet",
    contractId: process.env.CONTRACT_ID,
    identity: process.env.IDENTITY,
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
      case "--tvl":
        args.tvl = Number(next());
        break;
      case "--rate-bps":
        args.rateBps = Number(next());
        break;
      case "--reward-pool":
        args.rewardPool = Number(next());
        break;
      case "--fund":
        args.fund = Number(next());
        break;
      case "--decimals":
        args.decimals = Number(next());
        break;
      case "--months":
        args.horizonMonths = Number(next());
        break;
      case "--low-growth":
        args.lowGrowth = Number(next());
        break;
      case "--base-growth":
        args.baseGrowth = Number(next());
        break;
      case "--high-growth":
        args.highGrowth = Number(next());
        break;
      case "--contract-id":
        args.contractId = next();
        break;
      case "--network":
        args.network = next();
        break;
      case "--identity":
        args.identity = next();
        break;
      default:
        console.error(`Unknown argument: ${arg}`);
        args.help = true;
    }
  }

  return args;
}

function printHelp(): void {
  console.log(`Reward pool runway simulator

Usage:
  npx tsx scripts/runway-sim.ts [options]

Pool state — either pass these three manually, or pass --contract-id (or
set $CONTRACT_ID) to read them live via the Stellar CLI:
  --tvl <tokens>             Current total deposited, in whole tokens
  --rate-bps <bps>           Current reward_rate_bps
  --reward-pool <tokens>     Current reward pool balance, in whole tokens
  --decimals <n>             Token decimals for base-unit conversion on the
                             RPC path (default 7)

Simulation:
  --fund <tokens>            One-time top-up added to the reward pool before
                             simulating, to model a prospective
                             fund_reward_pool call (default 0)
  --months <n>               Projection horizon in months (default 24)
  --low-growth <rate>        Annual TVL growth, low scenario (default -0.25)
  --base-growth <rate>       Annual TVL growth, base scenario (default 0.5)
  --high-growth <rate>       Annual TVL growth, high scenario (default 2.0)

RPC lookup (same convention as scripts/pool.sh):
  --contract-id <id>         Soroban contract ID (or $CONTRACT_ID)
  --network <name>           Stellar network (or $NETWORK, default testnet)
  --identity <name>          Local Stellar identity used as --source (or $IDENTITY)

  -h, --help                 Show this help
`);
}

// ---------------------------------------------------------------------------
// RPC lookup via the Stellar CLI. `scripts/pool.sh` invokes this with a
// `--fn <name>` flag, which is rejected by the CLI version this was
// verified against (`stellar contract invoke --help`): the current syntax
// takes the function name and its own arguments positionally after a bare
// `--`, e.g. `stellar contract invoke --id ID --source ID --network NET
// --send=no -- vault_state`. `--source` is required even for a read-only
// simulated call. `--send=no` (replacing the deprecated `--is-view`) keeps
// this a simulation, never a signed/submitted transaction. Best-effort
// overall: the CLI's output format isn't guaranteed across versions, so
// this parses leniently and callers should fall back to manual
// --tvl/--rate-bps/--reward-pool on any failure.
// ---------------------------------------------------------------------------

function invokeReadOnly(
  contractId: string,
  network: string,
  identity: string,
  fn: string,
): string {
  const cliArgs = [
    "contract",
    "invoke",
    "--id",
    contractId,
    "--source",
    identity,
    "--network",
    network,
    "--send=no",
    "--",
    fn,
  ];
  return execFileSync("stellar", cliArgs, { encoding: "utf8" }).trim();
}

/** Extracts every digit run (optionally signed) from the CLI's raw output
 * and returns the last one as a number — robust to the handful of ways a
 * single i128/u32 return value or a tuple of them might be printed
 * (`25000000`, `"25000000"`, `[25000000, 100000000]`, `(…)`, …). */
function parseLastNumber(raw: string): number {
  const matches = raw.match(/-?\d+/g);
  if (!matches || matches.length === 0) {
    throw new Error(`Could not parse numeric contract output: ${raw}`);
  }
  return Number(matches[matches.length - 1]);
}

/**
 * Best-effort live read of pool state via the Stellar CLI, mirroring
 * `scripts/pool.sh`'s `pool-info` (`vault_state`, `get_reward_rate_bps`,
 * `get_reward_pool_balance`). Returns base-unit values — the caller
 * divides by `10 ** decimals`.
 */
function readPoolStateRawFromRpc(
  contractId: string,
  network: string,
  identity: string,
): { tvlRaw: number; rateBps: number; rewardPoolRaw: number } {
  // vault_state() -> (total_shares, total_deposited); TVL is the second element.
  const vaultState = invokeReadOnly(contractId, network, identity, "vault_state");
  const rateBpsRaw = invokeReadOnly(contractId, network, identity, "get_reward_rate_bps");
  const rewardPoolRaw = invokeReadOnly(contractId, network, identity, "get_reward_pool_balance");

  return {
    tvlRaw: parseLastNumber(vaultState),
    rateBps: parseLastNumber(rateBpsRaw),
    rewardPoolRaw: parseLastNumber(rewardPoolRaw),
  };
}

// ---------------------------------------------------------------------------
// Table rendering
// ---------------------------------------------------------------------------

function formatTable(results: ScenarioResult[], decimals: number): string {
  const months = results[0]?.monthlyBalances.length ?? 0;
  const header = ["Month", ...results.map((r) => `${r.name} (${(r.annualGrowthRate * 100).toFixed(0)}%/yr)`)];
  const rows: string[][] = [];
  for (let m = 0; m < months; m++) {
    rows.push([String(m), ...results.map((r) => r.monthlyBalances[m]!.toFixed(decimals))]);
  }

  const widths = header.map((h, i) => Math.max(h.length, ...rows.map((row) => row[i]!.length)));
  const renderRow = (cells: string[]) => cells.map((cell, i) => cell.padStart(widths[i]!)).join("  ");

  const lines = [renderRow(header), widths.map((w) => "-".repeat(w)).join("  ")];
  for (const row of rows) {
    lines.push(renderRow(row));
  }
  return lines.join("\n");
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

function main(): void {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    printHelp();
    return;
  }

  let state: PoolState;
  const haveManualInputs =
    args.tvl !== undefined && args.rateBps !== undefined && args.rewardPool !== undefined;

  if (haveManualInputs) {
    state = { tvl: args.tvl!, rateBps: args.rateBps!, rewardPool: args.rewardPool! };
  } else if (args.contractId) {
    if (!args.identity) {
      console.error("Reading pool state via RPC requires an identity: pass --identity or set $IDENTITY.");
      process.exitCode = 1;
      return;
    }
    console.error(`Reading live pool state for ${args.contractId} on ${args.network}...`);
    try {
      const raw = readPoolStateRawFromRpc(args.contractId, args.network, args.identity);
      const scale = 10 ** args.decimals;
      state = {
        tvl: raw.tvlRaw / scale,
        rateBps: raw.rateBps,
        rewardPool: raw.rewardPoolRaw / scale,
      };
    } catch (err) {
      console.error(`Failed to read pool state via the Stellar CLI: ${(err as Error).message}`);
      console.error("Pass --tvl, --rate-bps, and --reward-pool manually instead.");
      process.exitCode = 1;
      return;
    }
  } else {
    console.error(
      "Provide either --tvl/--rate-bps/--reward-pool, or --contract-id " +
        "(with $IDENTITY/$NETWORK) to read pool state from RPC.",
    );
    printHelp();
    process.exitCode = 1;
    return;
  }

  state.rewardPool += args.fund;

  const scenarios: Array<[string, number]> = [
    ["low", args.lowGrowth],
    ["base", args.baseGrowth],
    ["high", args.highGrowth],
  ];
  const results = scenarios.map(([name, growth]) => simulateScenario(name, state, growth, args.horizonMonths));

  const fundNote = args.fund ? ` (includes +${args.fund.toLocaleString()} simulated funding)` : "";
  console.log(
    `Pool state: TVL=${state.tvl.toLocaleString()} rate_bps=${state.rateBps} ` +
      `reward_pool=${state.rewardPool.toLocaleString()}${fundNote}\n`,
  );
  console.log(formatTable(results, 2));
  console.log();
  for (const result of results) {
    const verdict =
      result.exhaustionMonth === null
        ? `survives the full ${args.horizonMonths}-month horizon`
        : `exhausted around month ${result.exhaustionMonth}`;
    console.log(`${result.name.padEnd(5)} (${(result.annualGrowthRate * 100).toFixed(0)}%/yr TVL growth): ${verdict}`);
  }
}

main();
