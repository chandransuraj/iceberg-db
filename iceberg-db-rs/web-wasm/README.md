# iceberg-db in the browser (WASM)

## Build failed → HTTP 404

Trunk only serves pages after a **successful** WASM build. If `cargo` exits **101**, fix the build first.

### Windows: `clang` not found (most common)

Iceberg links **zstd**, which compiles C code for `wasm32`. You need **LLVM** and `clang` on `PATH`.

```powershell
winget install --id LLVM.LLVM -e
```

Then use the helper script (adds LLVM to PATH for this session):

```powershell
cd C:\Users\chand\.cursor\projects\empty-window\iceberg-db-rs\web-wasm
.\serve.ps1
```

Do **not** rely on an old terminal opened before LLVM was installed — either open a **new** terminal or use `serve.ps1`.

Verify manually:

```powershell
& "${env:ProgramFiles}\LLVM\bin\clang.exe" --version
```

## Prerequisites

```powershell
rustup target add wasm32-unknown-unknown
cargo install trunk
```

## Manual build (see full errors)

```powershell
$env:Path = "$env:ProgramFiles\LLVM\bin;$env:Path"
$env:CC_wasm32_unknown_unknown = "clang"
cd C:\Users\chand\.cursor\projects\empty-window\iceberg-db-rs
cargo build -p idb-wasm --target wasm32-unknown-unknown
```

First build: 10–20+ minutes.

## Native vs WASM

| | `idb-cli -w …` | Browser WASM |
|--|----------------|--------------|
| Data | Warehouse on disk | In-memory `demo.customers` (3 rows) |
