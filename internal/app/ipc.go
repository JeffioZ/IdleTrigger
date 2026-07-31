package app

import (
	"github.com/JeffioZ/idletrigger/internal/config"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/systemaction"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
)

func (s *runtimeState) handleIPC(cmd string) string {
	return s.call(func() string { return s.handleIPCState(cmd) })
}

func (s *runtimeState) handleIPCState(cmd string) string {
	mylog.Info("IPC command received: %q", cmd)
	switch cmd {
	case "sleep":
		if err := s.executeAction(config.ActionSleep); err != nil {
			return "err: " + err.Error()
		}
		return "ok"
	case "hibernate":
		if err := s.executeAction(config.ActionHibernate); err != nil {
			return "err: " + err.Error()
		}
		return "ok"
	case "shutdown":
		if err := systemaction.Shutdown(); err != nil {
			return "err: " + err.Error()
		}
		return "ok"
	case "lock":
		if err := systemaction.Lock(); err != nil {
			return "err: " + err.Error()
		}
		return "ok"

	case "nosleep:on":
		s.setNoSleep(true, false)
		if err := s.saveConfigErr(); err != nil {
			return "err: " + err.Error()
		}
		return "ok"
	case "nosleep:on:screen":
		s.setNoSleep(true, true)
		if err := s.saveConfigErr(); err != nil {
			return "err: " + err.Error()
		}
		return "ok"
	case "nosleep:off":
		s.setNoSleep(false, s.cfg.KeepScreenOn)
		if err := s.saveConfigErr(); err != nil {
			return "err: " + err.Error()
		}
		return "ok"
	case "nosleep:toggle":
		s.toggleNoSleep()
		return "ok"
	case "nosleep:status":
		return s.fmtNoSleepStatus()

	case "monitor:on":
		minutes := s.cfg.IdleTimeoutMinutes
		if minutes <= 0 {
			minutes = config.DefaultIdleTimeoutMinutes
		}
		s.setIdleTimeout(minutes)
		if err := s.saveConfigErr(); err != nil {
			return "err: " + err.Error()
		}
		return "ok"
	case "monitor:off":
		s.setIdleTimeout(0)
		if err := s.saveConfigErr(); err != nil {
			return "err: " + err.Error()
		}
		return "ok"
	case "monitor:status":
		return s.statusLine("status_monitor", s.monitorStatusText())

	case "status":
		return s.fmtStatus()

	case "ping":
		return "pong"
	case "open":
		if !trayicon.Post(s.showControlPanel) {
			return "err: tray UI is not ready"
		}
		return "ok"

	case "config:reload":
		mylog.Info("IPC config reload requested")
		if err := s.reloadConfig(); err != nil {
			return "err: " + err.Error()
		}
		return "ok"

	default:
		return "err: unknown command: " + cmd
	}
}
