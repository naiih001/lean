#!/usr/bin/env bash
set -e
echo "=== 1:1 lint: wc -l src/**/*.rs src/**/**/*.rs ==="
# Use find for portability
echo "--- wc -l sorted ---"
find src -name "*.rs" -exec wc -l {} + | sort -n | tail -n 30
echo ""
echo "--- files >550 (should be empty except tui/mod.rs shim) ---"
find src -name "*.rs" -exec wc -l {} + | sort -n | awk '$1>550 {print}'
echo ""
echo "--- files >350 (for info) ---"
find src -name "*.rs" -exec wc -l {} + | sort -n | awk '$1>350 {print}'
echo ""
echo "--- pub struct/enum per file (>2 suggests multi-topic) ---"
rg -n "^pub (struct|enum) " src --no-heading | cut -d: -f1 | sort | uniq -c | sort -rn | head -n 20
echo ""
echo "--- cargo fmt --check ---"
cargo fmt --check && echo "fmt ok" || echo "fmt needs run (cargo fmt)"
echo ""
echo "--- cargo clippy (warnings) ---"
cargo clippy 2>&1 | tail -n 20
echo ""
echo "--- cargo test ---"
cargo test 2>&1 | tail -n 10
