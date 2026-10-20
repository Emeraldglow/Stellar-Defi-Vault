#!/usr/bin/env tsx
/**
 * Validate deployment configuration files.
 *
 * Usage:
 *   npx tsx scripts/validate-config.ts config/testnet.toml
 *   npx tsx scripts/validate-config.ts config/mainnet.toml
 */

import * as fs from "fs";
import * as path from "path";

interface VaultConfig {
  network: {
    name: string;
    rpc_url: string;
    horizon_url: string;
    network_passphrase: string;
  };
  contract: {
    token_address: string;
  };
  vault: {
    management_fee_bps: number;
    performance_fee_bps: number;
    withdrawal_fee_bps: number;
    base_yield_rate_bps: number;
    bonus_yield_rate_bps: number;
    max_deposit_cap: number;
    max_withdrawal_cap: number;
    min_deposit_amount: number;
  };
  admin: {
    admin_addresses: string[];
    keeper_addresses: string[];
  };
  security: {
    pause_threshold: number;
    max_daily_withdrawal: number;
  };
}

// Validation limits
const LIMITS = {
  MAX_FEE_BPS: 1000, // 10% max fee
  MAX_RATE_BPS: 10000, // 100% max rate
  MAX_CAP: 10 ** 18,
  MIN_DEPOSIT: 1,
  MAX_ADMINS: 10,
  MAX_KEEPERS: 20,
};

function validateConfig(config: VaultConfig, configPath: string): string[] {
  const errors: string[] = [];
  const warnings: string[] = [];

  // Validate network
  if (!config.network?.name) {
    errors.push("network.name is required");
  }
  if (!config.network?.rpc_url?.startsWith("https://")) {
    errors.push("network.rpc_url must be a valid HTTPS URL");
  }
  if (!config.network?.horizon_url?.startsWith("https://")) {
    errors.push("network.horizon_url must be a valid HTTPS URL");
  }
  if (!config.network?.network_passphrase) {
    errors.push("network.network_passphrase is required");
  }

  // Validate contract
  if (!config.contract?.token_address) {
    errors.push("contract.token_address is required");
  } else if (!config.contract.token_address.match(/^C[A-Z2-7]{55}$/)) {
    errors.push("contract.token_address must be a valid Soroban contract address");
  }

  // Validate vault parameters
  const vault = config.vault;
  if (vault.management_fee_bps < 0 || vault.management_fee_bps > LIMITS.MAX_FEE_BPS) {
    errors.push(`vault.management_fee_bps must be between 0 and ${LIMITS.MAX_FEE_BPS}`);
  }
  if (vault.performance_fee_bps < 0 || vault.performance_fee_bps > LIMITS.MAX_FEE_BPS) {
    errors.push(`vault.performance_fee_bps must be between 0 and ${LIMITS.MAX_FEE_BPS}`);
  }
  if (vault.withdrawal_fee_bps < 0 || vault.withdrawal_fee_bps > LIMITS.MAX_FEE_BPS) {
    errors.push(`vault.withdrawal_fee_bps must be between 0 and ${LIMITS.MAX_FEE_BPS}`);
  }
  if (vault.base_yield_rate_bps < 0 || vault.base_yield_rate_bps > LIMITS.MAX_RATE_BPS) {
    errors.push(`vault.base_yield_rate_bps must be between 0 and ${LIMITS.MAX_RATE_BPS}`);
  }
  if (vault.bonus_yield_rate_bps < 0 || vault.bonus_yield_rate_bps > LIMITS.MAX_RATE_BPS) {
    errors.push(`vault.bonus_yield_rate_bps must be between 0 and ${LIMITS.MAX_RATE_BPS}`);
  }
  if (vault.max_deposit_cap <= 0 || vault.max_deposit_cap > LIMITS.MAX_CAP) {
    errors.push(`vault.max_deposit_cap must be between 1 and ${LIMITS.MAX_CAP}`);
  }
  if (vault.max_withdrawal_cap <= 0 || vault.max_withdrawal_cap > LIMITS.MAX_CAP) {
    errors.push(`vault.max_withdrawal_cap must be between 1 and ${LIMITS.MAX_CAP}`);
  }
  if (vault.min_deposit_amount < LIMITS.MIN_DEPOSIT) {
    errors.push(`vault.min_deposit_amount must be at least ${LIMITS.MIN_DEPOSIT}`);
  }
  if (vault.min_deposit_amount > vault.max_deposit_cap) {
    errors.push("vault.min_deposit_amount cannot exceed vault.max_deposit_cap");
  }

  // Validate admin
  const admin = config.admin;
  if (!admin.admin_addresses || admin.admin_addresses.length === 0) {
    errors.push("admin.admin_addresses must contain at least one address");
  } else if (admin.admin_addresses.length > LIMITS.MAX_ADMINS) {
    errors.push(`admin.admin_addresses cannot exceed ${LIMITS.MAX_ADMINS} addresses`);
  } else {
    for (const addr of admin.admin_addresses) {
      if (!addr.match(/^G[A-Z2-7]{55}$/)) {
        errors.push(`Invalid admin address: ${addr}`);
      }
    }
  }

  if (admin.keeper_addresses && admin.keeper_addresses.length > LIMITS.MAX_KEEPERS) {
    errors.push(`admin.keeper_addresses cannot exceed ${LIMITS.MAX_KEEPERS} addresses`);
  }

  // Validate security
  const security = config.security;
  if (security.pause_threshold < 1) {
    errors.push("security.pause_threshold must be at least 1");
  }
  if (security.pause_threshold > (admin.admin_addresses?.length || 0)) {
    warnings.push("security.pause_threshold exceeds number of admins");
  }
  if (security.max_daily_withdrawal <= 0) {
    errors.push("security.max_daily_withdrawal must be positive");
  }

  // Print results
  console.log(`\nValidating: ${configPath}\n`);

  if (errors.length > 0) {
    console.log("❌ Validation errors:");
    errors.forEach((e) => console.log(`   - ${e}`));
  }

  if (warnings.length > 0) {
    console.log("\n⚠️  Warnings:");
    warnings.forEach((w) => console.log(`   - ${w}`));
  }

  if (errors.length === 0) {
    console.log("✅ Configuration is valid!");
    console.log("\nSummary:");
    console.log(`   Network: ${config.network.name}`);
    console.log(`   Token: ${config.contract.token_address}`);
    console.log(`   Fees: mgmt=${vault.management_fee_bps}bps, perf=${vault.performance_fee_bps}bps, withdraw=${vault.withdrawal_fee_bps}bps`);
    console.log(`   Rates: base=${vault.base_yield_rate_bps}bps, bonus=${vault.bonus_yield_rate_bps}bps`);
    console.log(`   Caps: deposit=${vault.max_deposit_cap}, withdrawal=${vault.max_withdrawal_cap}`);
    console.log(`   Admins: ${admin.admin_addresses?.length || 0}`);
    console.log(`   Pause threshold: ${security.pause_threshold}`);
  }

  return errors;
}

// Simple TOML parser (basic implementation)
function parseToml(content: string): VaultConfig {
  const result: any = {};
  let currentSection = "";

  for (const line of content.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;

    const sectionMatch = trimmed.match(/^\[(.+)\]$/);
    if (sectionMatch) {
      currentSection = sectionMatch[1];
      result[currentSection] = {};
      continue;
    }

    const kvMatch = trimmed.match(/^(\w+)\s*=\s*(.+)$/);
    if (kvMatch && currentSection) {
      let [, key, value] = kvMatch;
      value = value.trim();

      // Remove comments
      const commentIndex = value.indexOf("#");
      if (commentIndex > 0) {
        value = value.substring(0, commentIndex).trim();
      }

      // Parse value
      if (value.startsWith('"') && value.endsWith('"')) {
        result[currentSection][key] = value.slice(1, -1);
      } else if (value.startsWith("[") && value.endsWith("]")) {
        // Simple array parsing
        const items = value.slice(1, -1).split(",").map((s) => s.trim().replace(/"/g, ""));
        result[currentSection][key] = items.filter((s) => s.length > 0);
      } else if (value === "true" || value === "false") {
        result[currentSection][key] = value === "true";
      } else {
        const num = Number(value);
        result[currentSection][key] = isNaN(num) ? value : num;
      }
    }
  }

  return result as VaultConfig;
}

// Main
const configPath = process.argv[2];
if (!configPath) {
  console.error("Usage: npx tsx scripts/validate-config.ts <config-path>");
  process.exit(1);
}

if (!fs.existsSync(configPath)) {
  console.error(`Config file not found: ${configPath}`);
  process.exit(1);
}

const content = fs.readFileSync(configPath, "utf-8");
const config = parseToml(content);
const errors = validateConfig(config, configPath);

process.exit(errors.length > 0 ? 1 : 0);
