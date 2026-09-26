# Releases / 发布

1. Run the development checks, build both architectures, and inspect PE imports for unexpected runtime dependencies.
2. Verify startup, second-instance activation, settings save/reload, task editing, process selection, both themes/languages, and keyboard navigation. Record the actual Windows/display environments tested.
3. Confirm diagnostic previews, forced themes, and capture behavior are unavailable in the release build. Use a disposable test environment for real system actions and theme repair.
4. Review changes and version before tagging. Tags follow `vMAJOR.MINOR.PATCH` with optional SemVer prerelease/build metadata. The three core components must each fit 0..65535; the fixed Windows version is `MAJOR.MINOR.PATCH.0`. ProductVersion retains the complete version, including labels and metadata.
5. The workflow validates, builds and inspects x64/x86 artifacts, calculates checksums, and creates a **draft** release. A prerelease label also sets GitHub's prerelease flag; build metadata alone does not. Review artifacts and write concise English and Chinese notes before publishing.

1. 执行开发检查、双架构构建，并检查 PE 导入表中的额外运行库依赖。
2. 验证启动、第二实例、配置保存及重载、任务编辑、进程选择、双主题语言和键盘导航，记录实际测试环境。
3. 确认发布版不启用诊断预览、强制主题和捕获行为；真实系统操作及主题修复使用独立测试环境。
4. 审查差异和版本后再打 tag。格式为 `vMAJOR.MINOR.PATCH`，可附 SemVer 预发布/构建信息；三个主版本分量各在 0..65535 内，Windows 固定版本为 `MAJOR.MINOR.PATCH.0`，ProductVersion 保留完整版本及附加信息。
5. 工作流完成验证、双架构构建、产物检查和校验和计算后创建**发布草稿**。有预发布标签时同时标记为 GitHub 预发布，只有构建信息时不标记。检查产物，填写简洁双语说明后发布。

Files: `IdleTrigger-x64.exe`, `IdleTrigger-x86.exe`, `SHA256SUMS.txt`. Describe user-visible changes and known limitations. Successful builds cannot replace unperformed hardware/UI checks.

Release notes use `## 更新重点` / `## Highlights` headings, group longer lists under bold subheadings (for example 行为变更 / 修复 / 改进), and end with the standard downloads line plus a Full Changelog compare link to the previous released tag. A release with no earlier tag omits the compare link.

发布文件如上。说明描述用户可见变化和已知限制，构建通过不能代替未执行的硬件或 UI 验证。

发布说明使用「更新重点 / Highlights」标题；条目较多时用加粗小节（如行为变更 / 修复 / 改进）分组；结尾附统一下载说明行和指向上一个已发布 tag 的完整变更比较链接。没有更早 tag 的首发版本省略比较链接。
