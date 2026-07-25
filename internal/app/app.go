// Package app coordinates IdleTrigger's serialized runtime state and features.
package app

import (
	"fmt"
	"github.com/JeffioZ/idletrigger/internal/config"
	"github.com/JeffioZ/idletrigger/internal/devtools"
	"github.com/JeffioZ/idletrigger/internal/feature/autorules"
	"github.com/JeffioZ/idletrigger/internal/feature/idle"
	"github.com/JeffioZ/idletrigger/internal/feature/keepawake"
	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	"github.com/JeffioZ/idletrigger/internal/i18n"
	mylog "github.com/JeffioZ/idletrigger/internal/logging"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/autostart"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/hotkey"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/ipc"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/powerstate"
	"github.com/JeffioZ/idletrigger/internal/ui/controlpanel"
	"github.com/JeffioZ/idletrigger/internal/ui/trayicon"
	"golang.org/x/sys/windows"
	"sync"
	"sync/atomic"
	"time"
)

const projectHomeURL = "https://github.com/JeffioZ/IdleTrigger"

var executeShell = windows.ShellExecute

func openWithShell(target, arguments string) error {
	verb, err := windows.UTF16PtrFromString("open")
	if err != nil {
		return err
	}
	file, err := windows.UTF16PtrFromString(target)
	if err != nil {
		return err
	}
	var args *uint16
	if arguments != "" {
		args, err = windows.UTF16PtrFromString(arguments)
		if err != nil {
			return err
		}
	}
	const swShowNormal = 1
	return executeShell(0, verb, file, args, nil, swShowNormal)
}

type Callbacks struct {
	OnConfigChanged         func(config.Config)
	ShowControlPanelOnStart bool
	DeveloperTools          devtools.Config
}

type runtimeState struct {
	cfg       config.Config
	lang      string
	callbacks Callbacks
	devtools  devtools.Config
	requestCh chan runtimeRequest

	mon *idle.Monitor // Owned by the serialized request loop; callbacks use post before accessing it.

	hotkeyMgr             *hotkey.Manager
	autoRunner            *autorules.Runner
	autoState             autorules.EffectiveState
	automationStatePath   string
	automationEvents      []autorules.Event
	automationWarningOpen bool
	automationGeneration  uint64
	themeSched            *theme.Scheduler
	themeSupportChecked   bool
	themeSupported        bool
	themeSupportErr       error
	batteryStop           chan struct{}
	batteryDone           chan struct{}
	ipLocationRetry       *time.Timer
	ipLocationGeneration  uint64
	ipLocationRetried     bool
	batteryBlocked        bool
	trayThemeDark         bool
	menuOpen              *trayicon.MenuItem
	menuExit              *trayicon.MenuItem
	selfConfigMod         atomic.Int64
	selfConfigWrite       atomic.Bool
	exiting               atomic.Bool
}

type runtimeRequest struct {
	fn     func() string
	result chan string
}

// Run starts the system-tray loop. Blocks until Exit.
func Run(cfg config.Config, cbs Callbacks) {
	trayicon.SetErrorHandler(func(format string, args ...interface{}) {
		mylog.Info("Systray: %s", fmt.Sprintf(format, args...))
	})

	s := &runtimeState{
		cfg:       cfg,
		lang:      cfg.Language,
		callbacks: cbs,
		devtools:  cbs.DeveloperTools,
		requestCh: make(chan runtimeRequest, 64),
	}
	s.detectThemeSupport()
	stateReady := make(chan struct{})
	var stateReadyOnce sync.Once
	go func() {
		<-stateReady
		s.requestLoop()
	}()
	var lifecycleMu sync.Mutex
	shuttingDown := false

	onReady := func() {
		lifecycleMu.Lock()
		defer lifecycleMu.Unlock()
		if shuttingDown {
			stateReadyOnce.Do(func() { close(stateReady) })
			return
		}
		defer stateReadyOnce.Do(func() { close(stateReady) })
		s.menuOpen = trayicon.AddMenuItem(i18n.T(s.lang, "menu_open_panel"), "")
		s.menuExit = trayicon.AddMenuItem(i18n.T(s.lang, "menu_exit"), "")
		go func() {
			for range s.menuOpen.ClickedCh {
				trayicon.Post(s.showControlPanel)
			}
		}()
		go func() {
			for range s.menuExit.ClickedCh {
				trayicon.Post(func() {
					hideAutomationUI()
					controlpanel.Destroy()
					trayicon.Quit()
				})
			}
		}()

		if enabled, updated, err := autostart.EnsureCurrent(); err == nil {
			s.cfg.AutostartEnabled = enabled
			if updated {
				mylog.Info("Autostart entry updated to current executable path")
			}
		} else {
			mylog.Info("Autostart check failed: %v", err)
		}

		// Config compatibility: if both NoSleep and idle monitor are enabled,
		// resolve the conflict — NoSleep takes priority.
		if s.cfg.NoSleepEnabled && s.cfg.IdleTimeoutMinutes > 0 {
			s.cfg.IdleTimeoutMinutes = 0
		}
		powerStatus := powerstate.GetStatus()
		s.batteryBlocked = batteryPolicyBlocks(s.cfg, powerStatus)
		s.logPowerState("startup", powerStatus)
		s.reconcileRuntime()

		s.updateIcon()

		// Hotkeys
		if s.cfg.HotkeysEnabled {
			s.startHotkeys()
		}

		s.startAutomation()
		if s.cfg.ThemeSwitchEnabled {
			s.startThemeScheduler()
		}
		s.startIPLocationCycle()

		go func() {
			if err := ipc.Server(s.handleIPC); err != nil {
				mylog.Info("IPC server stopped: %v", err)
			}
		}()
		s.syncBatteryLoop()
		go s.watchConfig()
		if cbs.ShowControlPanelOnStart {
			trayicon.Post(s.showControlPanel)
		}

		trayicon.SetOnLeftClick(func() { s.showControlPanel() })
		trayicon.SetOnPowerChange(func(event uint32) {
			s.post(func() { s.handlePowerEvent(event) })
		})
		trayicon.SetOnThemeChange(func() {
			s.post(s.refreshTrayThemeIcon)
		})
	}

	onExit := func() {
		s.exiting.Store(true)
		hideAutomationUI()
		lifecycleMu.Lock()
		defer lifecycleMu.Unlock()
		shuttingDown = true
		stateReadyOnce.Do(func() { close(stateReady) })
		s.call(func() string {
			s.stopIPLocationCycle()
			s.stopMonitor()
			s.stopHotkeys()
			s.stopAutomation()
			s.stopThemeScheduler()
			s.stopBatteryLoop()
			keepawake.Disable()
			return ""
		})
	}

	trayicon.Run(onReady, onExit)
}

func (s *runtimeState) requestLoop() {
	for req := range s.requestCh {
		result := req.fn()
		if req.result != nil {
			req.result <- result
		}
	}
}

func (s *runtimeState) post(fn func()) {
	s.postUntil(nil, fn)
}

func (s *runtimeState) postUntil(cancel <-chan struct{}, fn func()) bool {
	request := runtimeRequest{fn: func() string {
		fn()
		return ""
	}}
	if cancel == nil {
		s.requestCh <- request
		return true
	}
	select {
	case <-cancel:
		return false
	default:
	}
	select {
	case s.requestCh <- request:
		return true
	case <-cancel:
		return false
	}
}

func (s *runtimeState) call(fn func() string) string {
	result := make(chan string, 1)
	s.requestCh <- runtimeRequest{fn: fn, result: result}
	return <-result
}

// ---- state sync -------------------------------------------------------

// ---- icon state -------------------------------------------------------
