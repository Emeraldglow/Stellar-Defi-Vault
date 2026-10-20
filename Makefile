.PHONY: build test docs coverage coverage-lcov scout scout-sarif mutants deploy-testnet stake claim wasm-release fuzz-check fuzz-build fuzz interface-spec interface-spec-check

build:
	cargo build --target wasm32-unknown-unknown --release

# Release WASM with no test-only features (issue #658). `testutils` is not a
# default feature, so a plain release build must never pull test code in.
wasm-release:
	cargo build --target wasm32-unknown-unknown --release --no-default-features

test:
	cargo test --features testutils

# Line coverage summary via cargo-llvm-cov (issue #663). See docs/COVERAGE.md.
coverage:
	cargo llvm-cov --features testutils --summary-only

# Write lcov.info for editor coverage gutters.
coverage-lcov:
	cargo llvm-cov --features testutils --lcov --output-path lcov.info

# Scout static analysis (issue #664). See docs/STATIC_ANALYSIS.md.
scout:
	SCOUT_OFFLINE=1 cargo scout-audit

# Write scout.sarif for the triage script / report upload.
scout-sarif:
	SCOUT_OFFLINE=1 cargo scout-audit --output-format sarif

# Mutation testing scoped to the accounting core (issue #662).
# See docs/MUTATION_TESTING.md.
mutants:
	cargo mutants --config mutants.toml

# Build rustdoc and list undocumented public items (#675).
docs:
	cargo doc --no-deps --lib 2>&1 | tee target/rustdoc.log
	@echo "Undocumented public items: $$(grep -c 'missing documentation' target/rustdoc.log || true)"

deploy-testnet:
	chmod +x scripts/deploy_testnet.sh
	./scripts/deploy_testnet.sh

stake:
	@chmod +x scripts/stake.sh
	@if [ -z "$(AMOUNT)" ]; then \
		echo "Error: AMOUNT is required. Usage: make stake AMOUNT=10000000"; \
		exit 1; \
	fi
	./scripts/stake.sh $(AMOUNT)

claim:
	chmod +x scripts/claim.sh
	./scripts/claim.sh

# ── Public interface snapshot. See docs/INTERFACE_SPEC.md ──────────────────

# Check the committed snapshot (interface-spec/VaultContract.json) against
# a fresh build; fails with a diff if the public interface changed.
interface-spec-check:
	@chmod +x scripts/interface_spec.sh
	./scripts/interface_spec.sh

# Regenerate the committed snapshot from a fresh build. Review the diff,
# commit the result, and add a CHANGELOG.md entry for intentional changes.
interface-spec:
	@chmod +x scripts/interface_spec.sh
	./scripts/interface_spec.sh --update

# ── Fuzzing (issue #661). See fuzz/README.md ────────────────────────────────

# Fast check that the fuzz targets still compile (no nightly/libFuzzer needed).
fuzz-check:
	cargo check --manifest-path fuzz/Cargo.toml

# Build the libFuzzer binaries (requires nightly + cargo-fuzz).
fuzz-build:
	cargo +nightly fuzz build

# Run every target for FUZZ_TIME seconds each (default 60).
fuzz:
	cd fuzz && set -e; for t in $$(cargo +nightly fuzz list); do \
		echo "fuzzing $$t"; \
		cargo +nightly fuzz run $$t -- -max_total_time=$${FUZZ_TIME:-60}; \
	done
