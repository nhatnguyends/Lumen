"""Open the native Rust clock through the shared dock host (local debug build)."""
import sys
from run_native_dock import main

if __name__ == "__main__":
    sys.argv.insert(1, "--clock")
    raise SystemExit(main())
