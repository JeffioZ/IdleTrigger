# Rust 移植遗留事项 / Rust Port — Known Gaps & Open Items

Go→Rust 移植的已知差距与待办。对照基准：Go 仓库最后一次发布（`internal/ui/automationpanel`、`internal/ui/processpicker`、`internal/ui/nativeform`）。完成一项就划掉一项。

## 未解决的 Bug

- **picker/编辑器原生控件偶发"屏幕空白"**：进程列表（ListView）与输入框（EDIT）在部分机器上首次显示不合成到屏幕——数据存在（`LVM_GETITEMCOUNT` 有值、PrintWindow 可渲染），但屏幕像素为空。已加 `present_control`（Go `PresentControl` 等价）缓解，根因未定位。怀疑方向：首次揭示（DWM uncloak）与控件更新区域交互、特定显示器/DPI 组合。复现要点：真实鼠标路径打开，看屏幕（不要用 PrintWindow 截图判断）。

## 行为差距（用户可感知）

1. **进程说明不回传**：确认选择后，编辑器"i"信息对话框与预览卡只显示裸进程名；Go 会在确认时带回 `descriptions` 映射，显示"名称（说明）"（editor.go:493-505、loading_model.go:626-637）。
2. **picker 加载分相位缺失**：Go 有 loading/enriching/error 三相、加载中禁用刷新按钮、扫描失败显示错误文案（loading_model.go:173-247）；当前实现快照失败静默为空列表。另外激活时按快照新旧（15–60 秒）自动刷新（processpicker window_messages.go:508-511）。
3. **管理器选择按索引保持**：Go 跨重填按规则 ID 保持选中（manager.go:135-141）；当前按 `LB_GETCURSEL` 索引，列表顺序变化时会跳选。
4. **保存/启停/删除失败不回显**：Go 把保存错误写进管理器状态行或编辑器校验行（editor.go:647-668）；当前磁盘写失败只进日志。
5. **星期键盘导航**：Go 仅第一个星期按钮有 TabStop，方向键在七个按钮间移动，保存/取消创建顺序为取消在前（editor.go:98-105、controls_layout.go:571-597）。
6. **ListView 列宽不自适应**：Go 按客户区宽度重算三列（含 24px 滚动条预留、190/180 最小值，processpicker.go:903-943）；当前为固定 250/310/82。
7. **描述到达后重置滚动/焦点行**：Go 保存并恢复 top-index 与焦点行（loading_model.go:313-353）；当前全量重建回到顶部。

## 交互细节差距

8. **选择弹窗（choice popup）**：焦点恢复语义（选中后回焦按钮、Escape/F4 还原、焦点在属主内移动不关闭，choicepopup.go:500-544）；按下/释放行配对与按下行视觉（445-461）；超过可视行数时的主题滚动条与每格 1 行滚动（206-223）；从 BN_CLICKED 改为投递消息打开（editor.go:461-469）。
9. **主题切换后复选框位图不重建**：Go 的 `applyTheme` 重建 ListView 状态图（state_images.go）；当前仅创建时生成一次。
10. **编辑器外部变更冲突提示**：Go 的 `pendingState`/`automation_changed_external` 流程（automationpanel.go:578-614）未移植。

## 架构级已知差距（按需排期）

- 自动化三窗口的 tooltips（Go 为全部字段注册了 tip_automation_*）。
- 内容滚动（编辑器高于视口时的滚动条）与 WM_DPICHANGED 重建（Go nativeform scrollbar/viewport 体系）。
- ListView 原生表头/滚动条的主题化（Go 自绘表头 + 主题滚动条覆盖层）。

## 文档与产物

- Go 时代的 user guide / development / release-notes / roadmap / 截图已随移植删除（git 历史可查）；Rust 版文档按功能达标进度补回。
- `Cargo.lock` 已纳入版本管理（对齐 Go 仓库提交 go.sum 的做法）。
