# zellij-ime-per-pane

Zellij 插件，为每个 pane 独立保存和恢复输入法状态。

## 功能

在切换 pane 时自动保存当前输入法状态，并恢复目标 pane 之前使用的输入法。适合需要在不同 pane 中使用不同输入法的场景（例如一个 pane 写代码用英文输入法，另一个 pane 写文档用中文输入法）。

## 依赖

- [im-select](https://github.com/daipeihust/im-select) — 用于查询和切换输入法
- Rust 工具链（用于编译插件）

### 安装 im-select

```bash
# macOS
brew install im-select

# 或从源码安装
# https://github.com/daipeihust/im-select
```

确保 `im-select` 在 `PATH` 中可找到，或通过插件配置指定路径（见下方配置节）。

## 编译

```bash
rustup target add wasm32-wasip1
cargo build --release --target wasm32-wasip1
```

编译产物位于 `target/wasm32-wasip1/release/zellij_ime_per_pane.wasm`。

## 安装

将编译好的 WASM 文件复制到 Zellij 插件目录：

```bash
mkdir -p ~/.config/zellij/plugins
cp target/wasm32-wasip1/release/zellij_ime_per_pane.wasm ~/.config/zellij/plugins/
```

## 配置

在 Zellij 的 `config.kdl` 中添加插件加载配置：

```kdl
plugins {
    ime-per-pane location="file:~/.config/zellij/plugins/zellij_ime_per_pane.wasm"
}
```

### 自定义配置

在 `config.kdl` 中可以通过 `_configuration` 传参：

```kdl
plugins {
    ime-per-pane location="file:~/.config/zellij/plugins/zellij_ime_per_pane.wasm" {
        im_select "/opt/homebrew/bin/im-select"
        state_dir "~/.cache/zellij-ime"
    }
}
```

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `im_select` | `~/.local/bin/im-select` | im-select 可执行文件路径 |
| `state_dir` | `~/.cache/zellij-ime` | 输入法状态存储目录 |

## 工作原理

1. 插件监听 Zellij 的 `PaneUpdate` 事件
2. 当焦点切换到新 pane 时：
   - 保存当前 pane 的输入法状态到 `~/.cache/zellij-ime/{pane_id}.ime`
   - 如果目标 pane 有保存的状态，则恢复该输入法
3. 状态按 pane ID 持久化，重启 Zellij 后仍然有效

## 文件

| 路径 | 说明 |
|------|------|
| `~/.cache/zellij-ime/` | 输入法状态存储目录 |
| `~/.cache/zellij-ime/debug.log` | 调试日志 |
