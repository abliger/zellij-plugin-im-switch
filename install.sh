#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PLUGIN_NAME="zellij-ime-per-pane"
TARGET="wasm32-wasip1"
WASM_FILE="target/${TARGET}/release/${PLUGIN_NAME//-/_}.wasm"
ZELLIJ_PLUGINS_DIR="${HOME}/.config/zellij/plugins"

cd "${SCRIPT_DIR}"

# 1. 检查 cargo
if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo not found. Please install Rust: https://rustup.rs/"
    exit 1
fi

# 2. 检查并安装 wasm target
if ! rustup target list --installed | grep -q "^${TARGET}\$"; then
    echo "Target ${TARGET} not found, installing..."
    rustup target add "${TARGET}"
fi

# 3. 构建 release
echo "Building ${PLUGIN_NAME} (${TARGET})..."
cargo build --target "${TARGET}" --release

# 4. 检查产物
if [[ ! -f "${WASM_FILE}" ]]; then
    echo "Error: build failed, ${WASM_FILE} not found"
    exit 1
fi

# 5. 安装到 Zellij 插件目录
mkdir -p "${ZELLIJ_PLUGINS_DIR}"
cp "${WASM_FILE}" "${ZELLIJ_PLUGINS_DIR}/${PLUGIN_NAME}.wasm"

SIZE="$(du -h "${ZELLIJ_PLUGINS_DIR}/${PLUGIN_NAME}.wasm" | cut -f1)"
echo "Installed: ${ZELLIJ_PLUGINS_DIR}/${PLUGIN_NAME}.wasm (${SIZE})"
