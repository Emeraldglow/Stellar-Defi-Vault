import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { scValToNative, xdr } from "@stellar/stellar-sdk";

type Rule =
  | "pause"
  | "admin_change"
  | "large_withdrawal"
  | "fee_change"
  | "circuit_breaker_trip"
  | "low_reward_runway";

interface CursorState {
  startLedger: number;
  cursor?: string;
}

interface ContractEvent {
  cursor: string;
  ledger: number;
  topic: string[];
  value: unknown;
  txHash?: string;
}

interface EventsResult {
  events: ContractEvent[];
  latestLedger: number;
}

const ruleNames: Rule[] = [
  "pause",
  "admin_change",
  "large_withdrawal",
  "fee_change",
  "circuit_breaker_trip",
  "low_reward_runway",
];
const rpcUrl = requiredEnv("RPC_URL");
const contractId = requiredEnv("CONTRACT_ID");
const cursorPath = path.resolve(process.env.CURSOR_FILE ?? ".watch-events-cursor.json");
const cursorDir = path.dirname(cursorPath);
const pollMs = positiveInteger(process.env.POLL_INTERVAL_MS, 5_000);
const pageLimit = Math.min(positiveInteger(process.env.PAGE_LIMIT, 100), 1_000);
const webhookUrl = process.env.WEBHOOK_URL;
const configuredRules = (process.env.ALERT_RULES ?? ruleNames.join(","))
  .split(",")
  .map((rule) => rule.trim())
  .filter(Boolean);
const unknownRules = configuredRules.filter((rule) => !ruleNames.includes(rule as Rule));
if (unknownRules.length > 0) {
  throw new Error(`Unknown ALERT_RULES value(s): ${unknownRules.join(", ")}`);
}
const enabledRules = new Set<Rule>(configuredRules as Rule[]);
const thresholdEnv = process.env.LARGE_WITHDRAWAL_THRESHOLD;
if (enabledRules.has("large_withdrawal") && thresholdEnv === undefined) {
  throw new Error("Set LARGE_WITHDRAWAL_THRESHOLD to enable large-withdrawal alerts");
}
const largeWithdrawalThreshold = BigInt(thresholdEnv ?? "0");
if (enabledRules.has("large_withdrawal") && largeWithdrawalThreshold <= 0n) {
  throw new Error("LARGE_WITHDRAWAL_THRESHOLD must be a positive integer in token base units");
}
const startLedgerEnv = process.env.START_LEDGER;

function requiredEnv(name: string): string {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(`${name} must be set`);
  }
  return value;
}

function positiveInteger(value: string | undefined, fallback: number): number {
  if (value === undefined) return fallback;
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) {
    throw new Error(`Expected a positive integer, received ${value}`);
  }
  return parsed;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function decodeScVal(value: unknown): unknown {
  const encoded = typeof value === "string" ? value : isRecord(value) ? value.xdr : undefined;
  if (typeof encoded !== "string") {
    throw new Error("RPC event contains a malformed ScVal");
  }
  return scValToNative(xdr.ScVal.fromXDR(encoded, "base64"));
}

function topicName(encoded: string): string {
  const value = decodeScVal(encoded);
  return typeof value === "string" ? value : String(value);
}

function eventData(event: ContractEvent): unknown {
  return decodeScVal(event.value);
}

function eventTopics(event: ContractEvent): string[] {
  return event.topic.map(topicName);
}

function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function asBigInt(value: unknown): bigint | undefined {
  if (typeof value === "bigint") return value;
  if (typeof value === "number" && Number.isSafeInteger(value)) return BigInt(value);
  if (typeof value === "string" && /^-?\d+$/.test(value)) return BigInt(value);
  return undefined;
}

function json(value: unknown): string {
  return JSON.stringify(value, (_key, item: unknown) =>
    typeof item === "bigint" ? item.toString() : item,
  );
}

function alertFor(event: ContractEvent): string | undefined {
  const topics = eventTopics(event);
  const name = topics[0];
  const data = asArray(eventData(event));
  let rule: Rule | undefined;
  let title = name;

  if (enabledRules.has("pause") && ["paused", "unpaused", "auto_unp", "grd_paus"].includes(name)) {
    rule = "pause";
    title = `Vault ${name}`;
  } else if (enabledRules.has("admin_change") && ["admin_set", "emer_set", "emer_rvk", "grd_set", "grd_rm"].includes(name)) {
    rule = "admin_change";
    title = `Admin/guardian change: ${name}`;
  } else if (enabledRules.has("large_withdrawal") && ["withdraw", "emg_wdraw"].includes(name)) {
    const amount = asBigInt(data[name === "withdraw" ? 1 : 0]);
    if (amount !== undefined && amount >= largeWithdrawalThreshold) {
      rule = "large_withdrawal";
      title = `Large withdrawal (${amount.toString()} token base units)`;
    }
  } else if (
    enabledRules.has("fee_change") &&
    ["token_fee_set", "fee_set", "fee_queued", "fee_apld", "fee_chg"].includes(name)
  ) {
    rule = "fee_change";
    title = `Fee configuration changed: ${name}`;
  } else if (
    enabledRules.has("circuit_breaker_trip") &&
    ["cb_trip", "cb_set"].includes(name)
  ) {
    rule = "circuit_breaker_trip";
    title = name === "cb_trip" ? "Circuit breaker trip" : "Circuit breaker threshold changed";
  } else if (enabledRules.has("low_reward_runway") && ["rfil_alt", "rwpl_low"].includes(name)) {
    rule = "low_reward_runway";
    title = `Low reward runway: ${name}`;
  }

  if (!rule) return undefined;
  return json({
    alert: rule,
    title,
    contractId,
    ledger: event.ledger,
    txHash: event.txHash,
    topics,
    data,
  });
}

async function loadCursor(): Promise<CursorState> {
  try {
    const parsed: unknown = JSON.parse(await readFile(cursorPath, "utf8"));
    if (!isRecord(parsed)) {
      throw new Error(`Invalid cursor file ${cursorPath}`);
    }
    const startLedger = parsed.startLedger;
    if (
      typeof startLedger !== "number" ||
      !Number.isSafeInteger(startLedger) ||
      startLedger < 1 ||
      (parsed.cursor !== undefined && typeof parsed.cursor !== "string")
    ) {
      throw new Error(`Invalid cursor file ${cursorPath}`);
    }
    return {
      startLedger,
      ...(typeof parsed.cursor === "string" ? { cursor: parsed.cursor } : {}),
    };
  } catch (error) {
    if (isRecord(error) && error.code === "ENOENT") {
      if (startLedgerEnv === undefined) {
        throw new Error(`No cursor found; set START_LEDGER for the initial scan (${cursorPath})`);
      }
      const startLedger = positiveInteger(startLedgerEnv, 0);
      return { startLedger };
    }
    throw error;
  }
}

async function saveCursor(cursor: CursorState): Promise<void> {
  await mkdir(cursorDir, { recursive: true });
  const temporaryPath = `${cursorPath}.tmp`;
  await writeFile(temporaryPath, `${JSON.stringify(cursor, null, 2)}\n`, { mode: 0o600 });
  await rename(temporaryPath, cursorPath);
}

async function fetchEvents(cursor: CursorState): Promise<EventsResult> {
  const pagination: { limit: number; cursor?: string } = { limit: pageLimit };
  if (cursor.cursor) pagination.cursor = cursor.cursor;

  const response = await fetch(rpcUrl, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "getEvents",
      params: {
        startLedger: cursor.startLedger,
        filters: [{ type: "contract", contractIds: [contractId], topics: [["*"]] }],
        pagination,
      },
    }),
  });
  if (!response.ok) {
    throw new Error(`RPC getEvents returned HTTP ${response.status}`);
  }
  const body: unknown = await response.json();
  if (!isRecord(body)) throw new Error("RPC getEvents returned a malformed response");
  if (body.error !== undefined) {
    throw new Error(`RPC getEvents error: ${json(body.error)}`);
  }
  const result = body.result;
  if (!isRecord(result) || !Array.isArray(result.events) || typeof result.latestLedger !== "number") {
    throw new Error("RPC getEvents response is missing events or latestLedger");
  }

  const events = result.events.map((entry): ContractEvent => {
    const eventCursor = isRecord(entry)
      ? typeof entry.pagingToken === "string"
        ? entry.pagingToken
        : entry.id
      : undefined;
    if (
      !isRecord(entry) ||
      typeof eventCursor !== "string" ||
      typeof entry.ledger !== "number" ||
      !Array.isArray(entry.topic) ||
      !entry.topic.every((topic) => typeof topic === "string") ||
      entry.value === undefined
    ) {
      throw new Error("RPC getEvents returned a malformed contract event");
    }
    return {
      cursor: eventCursor,
      ledger: entry.ledger,
      topic: entry.topic,
      value: entry.value,
      ...(typeof entry.txHash === "string" ? { txHash: entry.txHash } : {}),
    };
  });
  return { events, latestLedger: result.latestLedger };
}

async function sendAlert(alert: string): Promise<void> {
  console.log(alert);
  if (!webhookUrl) return;
  const response = await fetch(webhookUrl, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ text: alert, content: alert }),
  });
  if (!response.ok) {
    throw new Error(`Alert webhook returned HTTP ${response.status}`);
  }
}

async function processPage(cursor: CursorState): Promise<boolean> {
  const page = await fetchEvents(cursor);
  for (const event of page.events) {
    const alert = alertFor(event);
    if (alert) await sendAlert(alert);
    cursor.cursor = event.cursor;
    await saveCursor(cursor);
  }

  if (page.events.length === pageLimit && page.events.length > 0) {
    cursor.cursor = page.events[page.events.length - 1].cursor;
    await saveCursor(cursor);
    return true;
  }

  cursor.startLedger = Math.max(cursor.startLedger, page.latestLedger + 1);
  delete cursor.cursor;
  await saveCursor(cursor);
  return false;
}

async function pollForever(): Promise<void> {
  const cursor = await loadCursor();
  console.log(
    `Watching ${contractId} from ledger ${cursor.startLedger}; enabled rules: ${
      [...enabledRules].join(", ") || "none"
    }; cursor file: ${cursorPath}`,
  );
  while (true) {
    try {
      while (await processPage(cursor)) {
        // Drain every full page before waiting for the next poll interval.
      }
    } catch (error) {
      console.error(
        `[${new Date().toISOString()}] event polling failed; cursor retained for retry: ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    }
    await new Promise((resolve) => setTimeout(resolve, pollMs));
  }
}

void pollForever().catch(async (error: unknown) => {
  const message = error instanceof Error ? error.stack ?? error.message : String(error);
  console.error(message);
  process.exitCode = 1;
});
