# ink → Rust 迁移手册（低 Bug 版本）

本文档用于约束迁移过程中的工程策略，目标是：

- 语义一致（与 C# ink 行为一致）
- 风险可控（每阶段可回退）
- 缺陷前置（尽可能在 CI 发现）

---

## 1. 分阶段迁移策略

### Phase 0（当前）
- 建立 Rust workspace
- 完成 JSON 读取 + 最小 runtime 骨架
- 建立 C# vs Rust 差分执行框架

### Phase 1（runtime 对齐）
- 补齐 `ControlCommand`、`Divert`、`ChoicePoint`、`CallStack`
- 将 `continue/choice` 行为与 C# 对齐
- 添加 save/load 与多 flow 的语义测试

### Phase 2（compiler 迁移）
- parser（递归下降）迁移
- AST 到 runtime JSON codegen
- 与 C# 编译产物做结构+行为双重对比

### Phase 3（CLI 和生态）
- `inklecate-rs` 参数兼容
- stats/json output/plugin 兼容
- 发布与版本策略落地

---

## 2. “不出 Bug”工程守则

## 2.1 差分优先
每个里程碑必须保留“同输入、双实现、比输出”机制。

最小比对项：
- 文本输出（`Continue` / `ContinueMaximally`）
- 选项列表（text/tags）
- 错误/警告

扩展比对项：
- visit/turn 计数
- callstack 深度与线程
- save/load 前后状态

## 2.2 严格模式
Rust 端遇到未实现语义时：
- 开发模式：记录 warning
- CI 严格模式：直接失败（防止“静默错误”）

## 2.3 回归资产复用
- 优先复用上游 `ink/tests/Tests.cs` 的语义场景
- 对每次修复添加回归测试，避免复发

## 2.4 禁止大爆炸替换
- 仅在“差分通过率达标”时替换路径
- 发布期保留 C# fallback

---

## 3. CI 建议

最低门槛：

1. `cargo fmt -- --check`
2. `cargo clippy --workspace -- -D warnings`
3. `cargo test --workspace`
4. 语义差分 smoke（选取一组固定 `.ink` 样例）

进阶门槛：

5. 属性测试（proptest）
6. Fuzz（`cargo fuzz`）
7. 跨平台 matrix（Windows / Linux / macOS）

---

## 4. 里程碑验收标准（建议）

### Runtime 替换准备就绪
- 核心语义差分通过率 ≥ 99%
- 已知差异清单可解释、可追踪
- 严格模式无新增回归

### Compiler 替换准备就绪
- 主流语法编译产物结构对齐
- 随机语料与真实项目语料通过

---

## 5. 开发节奏建议

每个 PR 建议只做一种类型变更：
- 语义补齐
- 差分修复
- 测试增强
- 重构（无行为变化）

并要求：
- 附带一个“差分前后”对比说明
- 新增或更新对应回归用例
