package config

import (
	"fmt"
	"strconv"
	"strings"

	"github.com/JeffioZ/idletrigger/internal/automation"
	"github.com/JeffioZ/idletrigger/internal/i18n"
)

func renderAnnotatedTOML(cfg Config) string {
	var b strings.Builder
	fmt.Fprintf(&b, "%s\n", configTemplateVersionMarker())
	fmt.Fprintf(&b, "# %s\n", i18n.T(cfg.Language, "config_header"))
	fmt.Fprintf(&b, "# %s\n\n", i18n.T(cfg.Language, "config_edit_hint"))

	b.WriteString("# -- 保持唤醒 / Stay Awake --\n")
	b.WriteString("# 手动启用保持唤醒，阻止系统自动睡眠 / Manually keep the system awake and prevent automatic sleep\n")
	fmt.Fprintf(&b, "nosleep_enabled = %t\n", cfg.NoSleepEnabled)
	b.WriteString("# 保持唤醒时同步保持屏幕常亮 / Keep the display on while Stay Awake is active\n")
	fmt.Fprintf(&b, "keep_screen_on = %t\n", cfg.KeepScreenOn)
	b.WriteString("# 电池供电时仍允许保持唤醒 / Allow Stay Awake while running on battery\n")
	fmt.Fprintf(&b, "nosleep_on_battery = %t\n", cfg.NoSleepOnBattery)
	b.WriteString("# 电池电量低于此百分比时强制关闭保持唤醒 / Force-disable Stay Awake below this battery percentage\n")
	fmt.Fprintf(&b, "nosleep_battery_threshold = %d\n", cfg.NoSleepBatteryThreshold)
	b.WriteString("\n")

	b.WriteString("# -- 自动任务 / Automatic Tasks --\n")
	b.WriteString("# 自动任务仅在 IdleTrigger 运行时生效；状态操作只临时覆盖运行状态，且只能执行内置操作，不会启动命令或脚本 / Automatic tasks work only while IdleTrigger is running; state actions are temporary runtime overrides, and only built-in actions are available—commands and scripts are never launched\n")
	fmt.Fprintf(&b, "automation_enabled = %t\n", cfg.AutomationEnabled)
	b.WriteString("# 任务建议通过控制界面创建；列表按进程名匹配全部同名实例，也可浏览选择指定 EXE，PID 和说明不会写入规则 / Create tasks from the control UI; list choices match all same-name instances, or Browse can select a specific EXE; PIDs and descriptions are not stored in rules\n")
	b.WriteString("# 进程启动任务只在所选进程由无到有时触发；退出任务需所有同名实例退出并经过 5 秒宽限 / Process-start tasks fire only when selected processes change from none to any; exit tasks wait for all same-name instances plus a 5-second grace period\n")
	b.WriteString("# Windows 睡眠期间错过的计划时间不会补执行；短于 5 秒扫描周期的进程可能无法捕获 / Scheduled times missed during Windows sleep are not replayed; processes shorter than the 5-second scan interval may be missed\n")
	b.WriteString("# 自动系统操作始终显示至少 10 秒的可取消倒计时 / Automatic system actions always show a cancellable countdown of at least 10 seconds\n")
	b.WriteString("\n")

	b.WriteString("# -- 空闲监测 / Idle Monitoring --\n")
	b.WriteString("# 空闲时长：无键鼠操作多少分钟后触发动作，设为 0 禁用 / Idle time in minutes before triggering after no keyboard or mouse input, 0 = disabled\n")
	fmt.Fprintf(&b, "idle_timeout_minutes = %d\n", cfg.IdleTimeoutMinutes)
	b.WriteString("# 达到空闲时长后执行的动作 / Action to run after the idle time is reached: \"sleep\", \"hibernate\", \"shutdown\", \"lock\"\n")
	fmt.Fprintf(&b, "idle_action = %s\n", tomlString(string(cfg.IdleAction)))
	b.WriteString("# 触发前多少秒显示不抢焦点的应用内提醒；键鼠操作或关闭提醒会取消本次动作，设为 0 静默执行 / Seconds before trigger to show a non-activating in-app reminder; keyboard/mouse input or closing it cancels this action, 0 = silent\n")
	fmt.Fprintf(&b, "idle_warning_seconds = %d\n", cfg.IdleWarningSeconds)
	b.WriteString("# 增强空闲监测：适合系统睡眠也被固定间隔空闲刷新干扰的机器；默认关闭，普通键鼠操作仍会重置计时 / Enhanced idle monitoring for machines where system sleep is disturbed by fixed-interval idle refreshes; off by default, and normal keyboard or mouse input still resets idle time\n")
	fmt.Fprintf(&b, "idle_enhanced_monitor = %t\n\n", cfg.IdleEnhancedMonitor)

	b.WriteString("# -- 昼夜主题 / Day/Night Theme --\n")
	b.WriteString("# 启用按时间自动切换 Windows 深浅色 / Automatically switch Windows light/dark theme by schedule\n")
	fmt.Fprintf(&b, "theme_switch_enabled = %t\n", cfg.ThemeSwitchEnabled)
	b.WriteString("# 切换模式：\"fixed\" 使用下方固定时间；\"sunrise\" 根据日出日落计算 / Mode: \"fixed\" uses times below; \"sunrise\" calculates sunrise/sunset\n")
	fmt.Fprintf(&b, "theme_mode = %s\n", tomlString(cfg.ThemeMode))
	b.WriteString("# 浅色开始时间，HH:MM；fixed 模式使用，日出日落不可用时也作为兜底 / Light theme start time, HH:MM; used by fixed mode and as fallback when sunrise/sunset is unavailable\n")
	fmt.Fprintf(&b, "theme_light_time = %s\n", tomlString(cfg.ThemeLightTime))
	b.WriteString("# 深色开始时间，HH:MM；fixed 模式使用，日出日落不可用时也作为兜底 / Dark theme start time, HH:MM; used by fixed mode and as fallback when sunrise/sunset is unavailable\n")
	fmt.Fprintf(&b, "theme_dark_time = %s\n", tomlString(cfg.ThemeDarkTime))
	b.WriteString("# 日出日落计算纬度，范围 -90 到 90；经纬度都为 0 时优先看下方 IP 定位开关，未开启或失败则按时区、UTC 偏移和默认位置依次回退 / Latitude for sunrise mode, -90..90; when both lat/lon are 0, the IP location option below is used first; otherwise falls back to timezone, UTC offset, then default location\n")
	fmt.Fprintf(&b, "theme_latitude = %s\n", tomlFloat(cfg.ThemeLatitude))
	b.WriteString("# 日出日落计算经度，范围 -180 到 180；经纬度都为 0 时优先看下方 IP 定位开关，未开启或失败则按时区、UTC 偏移和默认位置依次回退 / Longitude for sunrise mode, -180..180; when both lat/lon are 0, the IP location option below is used first; otherwise falls back to timezone, UTC offset, then default location\n")
	fmt.Fprintf(&b, "theme_longitude = %s\n", tomlFloat(cfg.ThemeLongitude))
	b.WriteString("# 经纬度都为 0 时，通过 https://ipwho.is/ 估算公网 IP 所在城市；成功结果仅内存缓存 24 小时，每次启动或手动重新开启定位时，首次失败后会在 30 分钟后补试一次；关闭或失败时按时区、UTC 偏移和默认位置依次回退 / When coordinates are 0, estimate city via https://ipwho.is/; successful results are cached in memory for 24 hours, and one retry is made 30 minutes after the first failure on app start or explicit re-enable; falls back to timezone, UTC offset, then default location when off or failed\n")
	fmt.Fprintf(&b, "theme_ip_location_enabled = %t\n", cfg.ThemeIPLocationEnabled)
	b.WriteString("# 电池供电时自动切换深色，接入电源后按当前计划恢复 / Switch to dark on battery, then restore by schedule on AC power\n")
	fmt.Fprintf(&b, "theme_dark_on_battery = %t\n", cfg.ThemeDarkOnBattery)
	b.WriteString("# 检测到全屏应用、演示模式或前台游戏活动时暂不自动切换主题 / Pause automatic theme switching during fullscreen apps, presentations, or foreground game activity\n")
	fmt.Fprintf(&b, "theme_skip_fullscreen = %t\n\n", cfg.ThemeSkipFullscreen)

	b.WriteString("# -- 设置 / Settings --\n")
	b.WriteString("# 启用全局热键：Ctrl+Win+Shift+S 睡眠，Win+Shift+L 锁定，Win+Shift+N 切换保持唤醒 / Enable global hotkeys: Ctrl+Win+Shift+S sleep, Win+Shift+L lock, Win+Shift+N toggle Stay Awake\n")
	fmt.Fprintf(&b, "hotkeys_enabled = %t\n", cfg.HotkeysEnabled)
	b.WriteString("# 将调试日志写入 EXE 同目录的 IdleTrigger.log；每行带启动会话标识 / Write debug logs to IdleTrigger.log next to the EXE; each line includes a startup session ID\n")
	fmt.Fprintf(&b, "logging_enabled = %t\n", cfg.LoggingEnabled)
	b.WriteString("# 界面语言：\"auto\" 跟随系统，\"en\" 英文，\"zh-CN\" 简体中文 / UI language: \"auto\" follows OS, \"en\" English, \"zh-CN\" Simplified Chinese\n")
	fmt.Fprintf(&b, "language = %s\n", tomlString(cfg.Language))

	// TOML array-table entries remain active until another table begins, so
	// rules must be written after every top-level setting.
	for _, rule := range cfg.AutomationRules {
		writeAutomationRule(&b, rule)
	}

	return b.String()
}

func tomlString(value string) string {
	var b strings.Builder
	b.Grow(len(value) + 2)
	b.WriteByte('"')
	for _, r := range value {
		switch r {
		case '"', '\\':
			b.WriteByte('\\')
			b.WriteRune(r)
		case '\b':
			b.WriteString(`\b`)
		case '\t':
			b.WriteString(`\t`)
		case '\n':
			b.WriteString(`\n`)
		case '\f':
			b.WriteString(`\f`)
		case '\r':
			b.WriteString(`\r`)
		default:
			if r < 0x20 || r == 0x7f {
				fmt.Fprintf(&b, `\u%04X`, r)
			} else {
				b.WriteRune(r)
			}
		}
	}
	b.WriteByte('"')
	return b.String()
}

func tomlStringList(values []string) string {
	if len(values) == 0 {
		return "[]"
	}
	quoted := make([]string, 0, len(values))
	for _, value := range values {
		quoted = append(quoted, tomlString(value))
	}
	return "[" + strings.Join(quoted, ", ") + "]"
}

func writeAutomationRule(b *strings.Builder, rule automation.Rule) {
	b.WriteString("\n[[automation_rules]]\n")
	fmt.Fprintf(b, "id = %s\n", tomlString(rule.ID))
	fmt.Fprintf(b, "name = %s\n", tomlString(rule.Name))
	fmt.Fprintf(b, "enabled = %t\n", rule.Enabled)
	fmt.Fprintf(b, "action = %s\n", tomlString(string(rule.Action)))
	fmt.Fprintf(b, "trigger = %s\n", tomlString(string(rule.Trigger)))
	if rule.Time != "" {
		fmt.Fprintf(b, "time = %s\n", tomlString(rule.Time))
	}
	if rule.EndTime != "" {
		fmt.Fprintf(b, "end_time = %s\n", tomlString(rule.EndTime))
	}
	if rule.Date != "" {
		fmt.Fprintf(b, "date = %s\n", tomlString(rule.Date))
	}
	if len(rule.Days) > 0 {
		fmt.Fprintf(b, "days = %s\n", tomlStringList(rule.Days))
	}
	if len(rule.Processes) > 0 {
		fmt.Fprintf(b, "process_logic = %s\n", tomlString(string(rule.ProcessLogic)))
		b.WriteString("processes = [")
		for index, target := range rule.Processes {
			if index > 0 {
				b.WriteString(", ")
			}
			fmt.Fprintf(b, "{ match = %s, executable = %s", tomlString(string(target.Match)), tomlString(target.Executable))
			if target.Match == automation.MatchPath {
				fmt.Fprintf(b, ", path = %s", tomlString(target.Path))
			}
			b.WriteString(" }")
		}
		b.WriteString("]\n")
	}
	if rule.KeepScreenOn {
		b.WriteString("keep_screen_on = true\n")
	}
	if rule.Action == automation.ActionEnableIdle {
		fmt.Fprintf(b, "idle_minutes = %d\n", rule.IdleMinutes)
	}
	if automation.IsEventAction(rule.Action) {
		fmt.Fprintf(b, "warning_seconds = %d\n", rule.WarningSeconds)
	}
	if rule.BlockedPolicy != "" {
		fmt.Fprintf(b, "blocked_policy = %s\n", tomlString(string(rule.BlockedPolicy)))
	}
	if rule.BlockedPolicy == automation.BlockedWait && rule.MaxWaitMinutes > 0 {
		fmt.Fprintf(b, "max_wait_minutes = %d\n", rule.MaxWaitMinutes)
	}
}

func tomlFloat(value float64) string {
	return strconv.FormatFloat(value, 'f', -1, 64)
}
