# ink-rs（迁移沙箱）

这是一个给 `ink` 做 Rust 改写的**增量迁移工作区**，目标不是一次性替换，而是：

1. 先建立 Rust 端的数据模型与运行时骨架；
2. 用差分测试持续对齐 C# 语义；
3. 在可回退前提下逐步替换组件。

> 当前状态：**Phase-0 / Scaffold**（可加载 `.ink.json` 并做最小文本继续；并提供 C# vs Rust 差分工具骨架）

---

## 工作区结构

- `crates/ink-model`：核心类型（InkDoc/Container/RuntimeNode/Error）
- `crates/ink-json`：ink runtime JSON 解析器（低层格式）
- `crates/ink-runtime`：最小运行时（先做线性 `continue` 骨架）
- `crates/inklecate-rs`：Rust CLI 原型（当前仅消费 `.json`）
- `crates/ink-diff-harness`：C# 与 Rust 的差分测试骨架

---

## 快速开始

```bash
cargo check --workspace
```

运行 Rust CLI（先对已编译 JSON）:

```bash
cargo run -p inklecate-rs -- path/to/story.ink.json -p --show-warnings
```

运行差分工具（调用原版 C# `inklecate` 编译+播放并对比）:

```bash
cargo run -p ink-diff-harness -- --ink ../ink/tests/test_included_file.ink --dump-output
```

---

## 低 Bug 原则（本仓库遵循）

1. **语义优先于重构美观**：先对齐行为，再谈性能和重构。
2. **差分即规范**：任何迁移改动都与 C# 结果对比。
3. **严格模式可提前失败**：发现未支持语义时，`--strict` 直接报错。
4. **分阶段替换**：runtime → compiler → CLI，始终保留回退路径。

详见：`docs/migration-playbook.md`
