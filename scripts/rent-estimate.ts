#!/usr/bin/env tsx
/**
 * Storage rent cost estimator.
 *
 * Estimates monthly rent costs based on user count and feature set.
 *
 * Usage:
 *   npx tsx scripts/rent-estimate.ts --users 1000
 *   npx tsx scripts/rent-estimate.ts --users 1000 --features auto-compound,keepers
 */

// Entry size estimates (bytes) derived from typed structs
const ENTRY_SIZES = {
  USER_SETTINGS: 256, // Consolidated user settings struct
  USER_BALANCE: 128, // User balance entry
  USER_HISTORY: 64, // Per-transaction history entry
  AUTO_COMPOUND: 96, // Auto-compound state
  KEEPERS: 128, // Keeper list (variable, estimated)
  GLOBAL_CONFIG: 512, // Global contract config
  POOL_STATE: 256, // Pool state entry
} as const;

// Network parameters (approximate)
const LEDGERS_PER_MONTH = 43_200; // 5s per ledger
const RENT_PER_BYTE_PER_LEDGER = 0.0000001; // Approximate XLM per byte per ledger

interface EstimateOptions {
  users: number;
  features: string[];
  includeHistory?: boolean;
}

function calculateRent(bytes: number, ledgers: number): number {
  return bytes * RENT_PER_BYTE_PER_LEDGER * ledgers;
}

function estimateStorage(options: EstimateOptions): void {
  const { users, features, includeHistory = false } = options;

  console.log(`\nStorage Rent Estimate for ${users.toLocaleString()} users`);
  console.log("=".repeat(50));

  let totalMonthlyRent = 0;

  // Base user entries
  console.log("\nBase User Entries:");
  console.log(`  User Settings (consolidated):`);
  console.log(`    Entries per user: 1`);
  console.log(`    Entry size: ${ENTRY_SIZES.USER_SETTINGS} bytes`);
  console.log(`    Total entries: ${users.toLocaleString()}`);
  const settingsBytes = ENTRY_SIZES.USER_SETTINGS * users;
  console.log(`    Total size: ${settingsBytes.toLocaleString()} bytes`);
  const settingsRent = calculateRent(settingsBytes, LEDGERS_PER_MONTH);
  console.log(`    Estimated monthly rent: ${settingsRent.toFixed(6)} XLM`);
  totalMonthlyRent += settingsRent;

  // User balance
  console.log(`\n  User Balance:`);
  console.log(`    Entries per user: 1`);
  console.log(`    Entry size: ${ENTRY_SIZES.USER_BALANCE} bytes`);
  console.log(`    Total entries: ${users.toLocaleString()}`);
  const balanceBytes = ENTRY_SIZES.USER_BALANCE * users;
  console.log(`    Total size: ${balanceBytes.toLocaleString()} bytes`);
  const balanceRent = calculateRent(balanceBytes, LEDGERS_PER_MONTH);
  console.log(`    Estimated monthly rent: ${balanceRent.toFixed(6)} XLM`);
  totalMonthlyRent += balanceRent;

  // Optional features
  if (features.includes("auto-compound")) {
    console.log(`\nAuto-Compound Feature:`);
    console.log(`  Additional entries per user: 1`);
    console.log(`  Entry size: ${ENTRY_SIZES.AUTO_COMPOUND} bytes`);
    console.log(`  Total entries: ${users.toLocaleString()}`);
    const acBytes = ENTRY_SIZES.AUTO_COMPOUND * users;
    console.log(`  Total size: ${acBytes.toLocaleString()} bytes`);
    const acRent = calculateRent(acBytes, LEDGERS_PER_MONTH);
    console.log(`  Estimated monthly rent: ${acRent.toFixed(6)} XLM`);
    totalMonthlyRent += acRent;
  }

  if (features.includes("keepers")) {
    console.log(`\nKeepers Feature:`);
    console.log(`  Additional entries per user: 1`);
    console.log(`  Entry size: ${ENTRY_SIZES.KEEPERS} bytes`);
    console.log(`  Total entries: ${users.toLocaleString()}`);
    const keeperBytes = ENTRY_SIZES.KEEPERS * users;
    console.log(`  Total size: ${keeperBytes.toLocaleString()} bytes`);
    const keeperRent = calculateRent(keeperBytes, LEDGERS_PER_MONTH);
    console.log(`  Estimated monthly rent: ${keeperRent.toFixed(6)} XLM`);
    totalMonthlyRent += keeperRent;
  }

  if (includeHistory) {
    console.log(`\nTransaction History:`);
    console.log(`  Entries per user (avg): 10`);
    console.log(`  Entry size: ${ENTRY_SIZES.USER_HISTORY} bytes`);
    const historyEntries = users * 10;
    console.log(`  Total entries: ${historyEntries.toLocaleString()}`);
    const historyBytes = ENTRY_SIZES.USER_HISTORY * historyEntries;
    console.log(`  Total size: ${historyBytes.toLocaleString()} bytes`);
    const historyRent = calculateRent(historyBytes, LEDGERS_PER_MONTH);
    console.log(`  Estimated monthly rent: ${historyRent.toFixed(6)} XLM`);
    totalMonthlyRent += historyRent;
  }

  // Global entries
  console.log(`\nGlobal Entries:`);
  const globalBytes = ENTRY_SIZES.GLOBAL_CONFIG + ENTRY_SIZES.POOL_STATE;
  console.log(`  Total size: ${globalBytes.toLocaleString()} bytes`);
  const globalRent = calculateRent(globalBytes, LEDGERS_PER_MONTH);
  console.log(`  Estimated monthly rent: ${globalRent.toFixed(6)} XLM`);
  totalMonthlyRent += globalRent;

  // Summary
  console.log("\n" + "=".repeat(50));
  console.log(`Total Estimated Monthly Rent: ${totalMonthlyRent.toFixed(6)} XLM`);
  console.log(`\nNote: This is an estimate. Actual costs depend on network fee rates.`);
  console.log(`      Use current network values for accurate calculations.`);
}

// Parse arguments
const args = process.argv.slice(2);
const usersIndex = args.indexOf("--users");
const featuresIndex = args.indexOf("--features");
const historyIndex = args.indexOf("--include-history");

if (usersIndex === -1) {
  console.error("Usage: npx tsx scripts/rent-estimate.ts --users <count> [--features <list>] [--include-history]");
  process.exit(1);
}

const users = parseInt(args[usersIndex + 1], 10);
if (isNaN(users) || users <= 0) {
  console.error("Invalid user count");
  process.exit(1);
}

const features = featuresIndex !== -1 ? args[featuresIndex + 1].split(",") : [];
const includeHistory = historyIndex !== -1;

estimateStorage({ users, features, includeHistory });
