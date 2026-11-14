# Native Lumen workspace

Migrated from D:/CustomTools/Widget to D:/CustomTools/Lumen on 2026-11-13.

Run by double-clicking Launch Lumen.pyw or using python run_lumen.py.

Includes current native Rust source, all widget packages, config (including legacy migration data, notes and music profiles), assets, docs, licenses, native portable archives, local Rust toolchain and Node adapter dependencies.

Old Electron experiments, desktop shortcuts pointing to the old workspace, and native target build cache were excluded. The old Widget directory remains intact as a backup. New builds regenerate under native-dock/target/debug. Nothing was published to GitHub.

Local copy/hash verification is in .scratch/migration-verification.json; this contains private local file paths and must remain untracked.

Verification: 13,410 files copied (6,636 MiB), 8,108 source/data files hash-verified, no copy errors or source drift. Offline native build succeeded. Native tests: 233 passed / 10 ignored. JavaScript safety/music tests: 15 passed. Python startup tests: 3 passed. Running source process verified from the new target/debug folder.
