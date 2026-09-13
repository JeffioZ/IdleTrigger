# Releases / 发布

1. Run the development checks, build both architectures, and inspect PE imports for unexpected runtime dependencies.
2. Verify startup, second-instance activation, settings save/reload, task editing, process selection, both themes/languages, and keyboard navigation. Record the actual Windows/display environments tested.
3. Confirm diagnostic previews, forced themes, and capture behavior are unavailable in the release build. Use a disposable test environment for real system actions and theme repair.
4. Review changes and version before tagging. Tags follow `vMAJOR.MINOR.PATCH` with optional SemVer prerelease/build metadata. Numeric resource components must fit Windows 16-bit version fields.
5. The workflow validates, builds x64/x86, calculates checksums, and creates a **draft** release. Review artifacts and write concise English and Chinese notes before publishing.

1. 执行开发检查、双架构构建，并检查 PE 导入表中的额外运行库依赖。
2. 验证启动、第二实例、配置保存及重载、任务编辑、进程选择、双主题语言和键盘导航，记录实际测试环境。
3. 确认发布版不启用诊断预览、强制主题和捕获行为；真实系统操作及主题修复使用独立测试环境。
4. 审查差异和版本后再打 tag。格式为 `vMAJOR.MINOR.PATCH`，可附 SemVer 预发布/构建信息，资源数值分量需在 Windows 16 位范围内。
5. 工作流完成验证、双架构构建和校验和计算后创建**发布草稿**。检查产物，填写简洁双语说明后发布。

Files: `IdleTrigger-x64.exe`, `IdleTrigger-x86.exe`, `SHA256SUMS.txt`. Describe user-visible changes and known limitations. Successful builds cannot replace unperformed hardware/UI checks.

发布文件如上。说明描述用户可见变化和已知限制，构建通过不能代替未执行的硬件或 UI 验证。
