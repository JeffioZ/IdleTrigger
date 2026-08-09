# Release Notes Style / 更新说明格式

[Documentation / 文档索引](README.md) · [Project Home](../README.md) · [项目主页](../README.zh-CN.md)

Write for people deciding whether to update, not as a copy of the commit log. / 面向正在判断是否升级的用户编写，不要复制提交记录。

## Editorial Rules / 编写原则

- Lead with user-visible results, not commit order. / 先写用户可感知的结果，不按提交顺序。
- Use plain-text headings without decorative emoji. / 标题只用文字，不添加装饰性图标。
- With one to three highlights, list them directly. With four to eight, use two or three bold product-area labels. / 只有 1–3 条时直接列出；有 4–8 条时，使用 2–3 个粗体产品模块标签。
- Prefer Automatic Tasks, Day / Night, Interface, and Other Changes over Added / Changed / Fixed. / 优先使用“自动任务”“昼夜主题”“界面”“其他变化”，不按“新增 / 变更 / 修复”机械分类。
- Keep one result in each bullet. Use a short sentence; split long or compound items. / 每条只说一个结果，尽量只用一个短句；内容过长或包含多个结果时拆开。
- Be specific: “preserves the selection while refreshing” is better than “improved refresh logic.” / 写清结果：“刷新时保留选择”优于“优化刷新逻辑”。
- Keep one to eight aligned highlights per language. Translate meaning, not sentence structure. / 每种语言保留 1–8 条对应重点；翻译含义，不照搬句式。
- A pre-release covers changes since the previous pre-release. A stable release may summarize changes since the previous stable version. / 预发布版只写相对上一个预发布版的增量；稳定版可汇总相对上一个稳定版的变化。
- Put required actions or compatibility notes first. Add one screenshot only when it helps explain a visible change. / 必要操作和兼容性说明放在前面；只有截图能解释可见变化时才添加。
- Do not repeat the asset list or routine checksum steps. GitHub and the user guide already provide them. / 不重复附件清单和常规校验步骤；GitHub 与使用指南已经提供这些信息。
- Leave PR lists and the comparison link at the end. / PR 明细和完整对比链接放在末尾。

## Recommended Structure / 推荐结构

Copy the following structure into the draft release. Omit the bold labels when there are no more than three highlights.

将以下结构复制到 Release 草稿中。更新不超过 3 条时，省略粗体分组标签。

````markdown
## 更新重点

**自动任务**

- 说明用户现在能做什么，或哪个常见问题已解决。

**界面**

- 描述可见结果；只有截图确实有帮助时才放在本分类后。

## Highlights

**Automatic Tasks**

- Explain what users can now do, or which common problem is resolved.

**Interface**

- Describe the visible result; add a screenshot after this category only when useful.

**完整变更 / Full Changelog:** https://github.com/JeffioZ/IdleTrigger/compare/vPREVIOUS...vCURRENT
````

## Final Check / 发布前检查

- The title is `IdleTrigger vX.Y.Z`, and the release is still a draft while editing. / 标题为 `IdleTrigger vX.Y.Z`，编辑期间保持草稿状态。
- Language headings and optional product labels use plain text, with no decorative emoji. / 语言标题和可选产品标签只用文字，不添加装饰性图标。
- Chinese and English highlights describe the same behavior. / 中英文亮点表达同一组行为。
- Asset names, architectures, and the comparison range are correct. / 产物名称、架构和版本对比范围正确。
- Every claim is supported by the release diff or completed verification. / 每条表述都能从版本差异或已完成验证中得到支持。
- Mention internal work only when it materially changes compatibility, reliability, security, size, or performance. / 内部工作仅在确实影响兼容性、可靠性、安全性、体积或性能时进入重点说明。
