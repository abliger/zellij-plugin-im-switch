# zellij-ime-per-pane

Zellij 插件，为每个 pane 独立保存和恢复输入法状态。

## 功能

在切换 pane 时自动保存当前输入法状态，并恢复目标 pane 之前使用的输入法。适合需要在不同 pane 中使用不同输入法的场景（例如一个 pane 写代码用英文输入法，另一个 pane 写文档用中文输入法）。

按 session 隔离状态，多个 Zellij session 互不干扰。

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

或使用自带的 install 脚本：

```bash
./install.sh
```

## 配置

在 Zellij 的 `config.kdl` 中添加插件加载配置：

```kdl
plugins {
    ime-per-pane location="file:/home/username/.config/zellij/plugins/zellij_ime_per_pane.wasm"
}
```

注意：`location` 中的路径由 Zellij 直接解析，不支持 `~` 展开，请使用绝对路径。

### 自定义配置

在 `config.kdl` 中可以通过 `_configuration` 传参：

```kdl
plugins {
    ime-per-pane location="file:/home/username/.config/zellij/plugins/zellij_ime_per_pane.wasm" {
        im_select "/pathto/im-select"
        state_dir "~/.cache/zellij-ime"
    }
}
```

`_configuration` 中的路径（如 `im_select`、`state_dir`）由插件自行解析，支持 `~` 展开。

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `im_select` | `~/.local/bin/im-select` | im-select 可执行文件路径 |
| `state_dir` | `~/.cache/zellij-ime` | 输入法状态存储目录 |

## 工作原理

1. 插件加载时请求 `ReadApplicationState` 和 `RunCommands` 权限
2. 订阅 `TabUpdate`、`PaneUpdate`、`ModeUpdate`、`SessionUpdate` 事件
3. 从 `ModeUpdate` 或 `SessionUpdate` 中提取当前 session 名称
4. 当焦点切换到新 pane 时：
   - 保存当前 pane 的输入法状态到 `~/.cache/zellij-ime/{session_name}/{pane_id}.ime`
   - 如果目标 pane 在该 session 中有保存的状态，则恢复该输入法
5. 插件加载时自动清理已不存在 session 的遗留状态目录

### Session 隔离

状态按 session 名称分目录存储：

```
~/.cache/zellij-ime/
├── session-a/
│   ├── t1.ime
│   └── t2.ime
└── session-b/
    ├── t1.ime
    └── t3.ime
```

- 同一 pane ID 在不同 session 中状态相互独立
- 当 session 被删除后，对应的状态目录会在下次 `SessionUpdate` 事件到达时自动清理

## 文件

| 路径 | 说明 |
|------|------|
| `~/.cache/zellij-ime/{session_name}/` | 输入法状态存储目录 |
