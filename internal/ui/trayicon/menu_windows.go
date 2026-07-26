package trayicon

import (
	"fmt"
	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/darkmode"
	"golang.org/x/sys/windows"
	"sort"
	"syscall"
	"unsafe"
)

func (t *winTray) createMenu() error {
	const (
		MIM_STYLE           = 0x00000010
		MIM_APPLYTOSUBMENUS = 0x80000000 // Settings apply to the menu and all of its submenus.
		MNS_NOCHECK         = 0x80000000 // No checkbox column: this menu contains plain commands only.
	)

	menuHandle, _, err := pCreatePopupMenu.Call()
	if menuHandle == 0 {
		return err
	}
	t.menus[0] = windows.Handle(menuHandle)

	// https://msdn.microsoft.com/en-us/library/windows/desktop/ms647575(v=vs.85).aspx
	mi := struct {
		Size, Mask, Style, Max uint32
		Background             windows.Handle
		ContextHelpID          uint32
		MenuData               uintptr
	}{
		Mask:  MIM_STYLE | MIM_APPLYTOSUBMENUS,
		Style: MNS_NOCHECK,
	}
	mi.Size = uint32(unsafe.Sizeof(mi))

	res, _, err := pSetMenuInfo.Call(
		uintptr(t.menus[0]),
		uintptr(unsafe.Pointer(&mi)),
	)
	if res == 0 {
		return err
	}
	return nil
}

func (t *winTray) convertToSubMenu(menuItemId uint32) (windows.Handle, error) {
	const MIIM_SUBMENU = 0x00000004

	res, _, err := pCreateMenu.Call()
	if res == 0 {
		return 0, err
	}
	menu := windows.Handle(res)

	mi := menuItemInfo{Mask: MIIM_SUBMENU, SubMenu: menu}
	mi.Size = uint32(unsafe.Sizeof(mi))
	t.muMenuOf.RLock()
	hMenu := t.menuOf[menuItemId]
	t.muMenuOf.RUnlock()
	res, _, err = pSetMenuItemInfo.Call(
		uintptr(hMenu),
		uintptr(menuItemId),
		0,
		uintptr(unsafe.Pointer(&mi)),
	)
	if res == 0 {
		return 0, err
	}
	t.muMenus.Lock()
	t.menus[menuItemId] = menu
	t.muMenus.Unlock()
	return menu, nil
}

func (t *winTray) addOrUpdateMenuItem(menuItemId uint32, parentId uint32, title string, disabled, checked bool) error {
	// https://msdn.microsoft.com/en-us/library/windows/desktop/ms647578(v=vs.85).aspx
	const (
		MIIM_FTYPE   = 0x00000100
		MIIM_BITMAP  = 0x00000080
		MIIM_STRING  = 0x00000040
		MIIM_SUBMENU = 0x00000004
		MIIM_ID      = 0x00000002
		MIIM_STATE   = 0x00000001
	)
	const MFT_STRING = 0x00000000
	const (
		MFS_CHECKED  = 0x00000008
		MFS_DISABLED = 0x00000003
	)
	titlePtr, err := windows.UTF16PtrFromString(title)
	if err != nil {
		return err
	}

	mi := menuItemInfo{
		Mask:     MIIM_FTYPE | MIIM_STRING | MIIM_ID | MIIM_STATE,
		Type:     MFT_STRING,
		ID:       uint32(menuItemId),
		TypeData: titlePtr,
		Cch:      uint32(len(title)),
	}
	mi.Size = uint32(unsafe.Sizeof(mi))
	if disabled {
		mi.State |= MFS_DISABLED
	}
	if checked {
		mi.State |= MFS_CHECKED
	}
	t.muMenuItemIcons.RLock()
	hIcon := t.menuItemIcons[menuItemId]
	t.muMenuItemIcons.RUnlock()
	if hIcon > 0 {
		mi.Mask |= MIIM_BITMAP
		mi.BMPItem = hIcon
	}

	var res uintptr
	t.muMenus.RLock()
	menu, exists := t.menus[parentId]
	t.muMenus.RUnlock()
	if !exists {
		menu, err = t.convertToSubMenu(parentId)
		if err != nil {
			return err
		}
		t.muMenus.Lock()
		t.menus[parentId] = menu
		t.muMenus.Unlock()
	} else if t.getVisibleItemIndex(parentId, menuItemId) != -1 {
		// We set the menu item info based on the menuID
		res, _, _ = pSetMenuItemInfo.Call(
			uintptr(menu),
			uintptr(menuItemId),
			0,
			uintptr(unsafe.Pointer(&mi)),
		)
	}

	if res == 0 {
		// Menu item does not already exist, create it
		t.muMenus.RLock()
		submenu, exists := t.menus[menuItemId]
		t.muMenus.RUnlock()
		if exists {
			mi.Mask |= MIIM_SUBMENU
			mi.SubMenu = submenu
		}
		t.addToVisibleItems(parentId, menuItemId)
		position := t.getVisibleItemIndex(parentId, menuItemId)
		res, _, err = pInsertMenuItem.Call(
			uintptr(menu),
			uintptr(position),
			1,
			uintptr(unsafe.Pointer(&mi)),
		)
		if res == 0 {
			t.delFromVisibleItems(parentId, menuItemId)
			return err
		}
		t.muMenuOf.Lock()
		t.menuOf[menuItemId] = menu
		t.muMenuOf.Unlock()
	}

	return nil
}

func (t *winTray) addSeparatorMenuItem(menuItemId, parentId uint32) error {
	// https://msdn.microsoft.com/en-us/library/windows/desktop/ms647578(v=vs.85).aspx
	const (
		MIIM_FTYPE = 0x00000100
		MIIM_ID    = 0x00000002
		MIIM_STATE = 0x00000001
	)
	const MFT_SEPARATOR = 0x00000800

	mi := menuItemInfo{
		Mask: MIIM_FTYPE | MIIM_ID | MIIM_STATE,
		Type: MFT_SEPARATOR,
		ID:   uint32(menuItemId),
	}

	mi.Size = uint32(unsafe.Sizeof(mi))

	t.addToVisibleItems(parentId, menuItemId)
	position := t.getVisibleItemIndex(parentId, menuItemId)
	t.muMenus.RLock()
	menu := uintptr(t.menus[parentId])
	t.muMenus.RUnlock()
	res, _, err := pInsertMenuItem.Call(
		menu,
		uintptr(position),
		1,
		uintptr(unsafe.Pointer(&mi)),
	)
	if res == 0 {
		return err
	}

	return nil
}

func (t *winTray) hideMenuItem(menuItemId, parentId uint32) error {
	// https://docs.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-removemenu
	const MF_BYCOMMAND = 0x00000000
	const ERROR_SUCCESS syscall.Errno = 0

	t.muMenus.RLock()
	menu := uintptr(t.menus[parentId])
	t.muMenus.RUnlock()
	res, _, err := pRemoveMenu.Call(
		menu,
		uintptr(menuItemId),
		MF_BYCOMMAND,
	)
	if res == 0 && err.(syscall.Errno) != ERROR_SUCCESS {
		return err
	}
	t.delFromVisibleItems(parentId, menuItemId)

	return nil
}

func (t *winTray) showMenu() error {
	const (
		TPM_BOTTOMALIGN = 0x0020
		TPM_LEFTALIGN   = 0x0000
		WM_NULL         = 0x0000
	)
	p := point{}
	res, _, err := pGetCursorPos.Call(uintptr(unsafe.Pointer(&p)))
	if res == 0 {
		return err
	}
	window := t.window
	pSetForegroundWindow.Call(uintptr(window))
	darkmode.SetPreferredAppMode(theme.Current() == theme.ModeDark)
	darkmode.RefreshMenuThemes()

	defer pPostMessage.Call(uintptr(window), WM_NULL, 0, 0)
	res, _, err = pTrackPopupMenu.Call(
		uintptr(t.menus[0]),
		TPM_BOTTOMALIGN|TPM_LEFTALIGN,
		uintptr(p.X),
		uintptr(p.Y),
		0,
		uintptr(window),
		0,
	)
	if res == 0 {
		return err
	}

	return nil
}

func (t *winTray) delFromVisibleItems(parent, val uint32) {
	t.muVisibleItems.Lock()
	defer t.muVisibleItems.Unlock()
	visibleItems := t.visibleItems[parent]
	for i, itemval := range visibleItems {
		if val == itemval {
			t.visibleItems[parent] = append(visibleItems[:i], visibleItems[i+1:]...)
			break
		}
	}
}

func (t *winTray) addToVisibleItems(parent, val uint32) {
	t.muVisibleItems.Lock()
	defer t.muVisibleItems.Unlock()
	if visibleItems, exists := t.visibleItems[parent]; !exists {
		t.visibleItems[parent] = []uint32{val}
	} else {
		newvisible := append(visibleItems, val)
		sort.Slice(newvisible, func(i, j int) bool { return newvisible[i] < newvisible[j] })
		t.visibleItems[parent] = newvisible
	}
}

func (t *winTray) getVisibleItemIndex(parent, val uint32) int {
	t.muVisibleItems.RLock()
	defer t.muVisibleItems.RUnlock()
	for i, itemval := range t.visibleItems[parent] {
		if val == itemval {
			return i
		}
	}
	return -1
}

// loadIconResource loads and caches an owned HICON from the executable's
// RT_GROUP_ICON resources. LoadImageW uses an exact frame when one exists and
// otherwise scales the nearest resource to the Shell-reported dimensions.
func (t *winTray) loadIconResource(key loadedImageKey) (windows.Handle, error) {
	const IMAGE_ICON = 1
	if key.width == 0 || key.height == 0 {
		return 0, fmt.Errorf("invalid icon size %dx%d", key.width, key.height)
	}

	// Serialize the load with shutdown and with other loads of the same key.
	// This keeps one owned HICON per key and never writes into a released cache.
	t.muLoadedImages.Lock()
	defer t.muLoadedImages.Unlock()
	if t.iconsReleased || t.loadedImages == nil {
		return 0, errTrayUnavailable
	}
	if h, ok := t.loadedImages[key]; ok {
		return h, nil
	}
	res, _, err := pLoadImage.Call(
		uintptr(t.instance),
		uintptr(key.resourceID),
		IMAGE_ICON,
		uintptr(key.width),
		uintptr(key.height),
		0,
	)
	if res == 0 {
		return 0, err
	}
	h := windows.Handle(res)
	t.loadedImages[key] = h
	return h, nil
}

func (t *winTray) loadMenuIconBitmap(key loadedImageKey) (windows.Handle, error) {
	t.muIconLifecycle.Lock()
	defer t.muIconLifecycle.Unlock()
	if !t.uiAvailable() {
		return 0, errTrayUnavailable
	}
	icon, err := t.loadIconResource(key)
	if err != nil {
		return 0, err
	}
	return t.iconToBitmap(icon)
}

func systemSmallIconKey(resourceID uint16) (loadedImageKey, error) {
	const (
		smCxSmallIcon = 49
		smCySmallIcon = 50
	)
	cx, _, _ := pGetSystemMetrics.Call(smCxSmallIcon)
	cy, _, _ := pGetSystemMetrics.Call(smCySmallIcon)
	if cx == 0 || cy == 0 {
		return loadedImageKey{}, fmt.Errorf("get system small icon size")
	}
	return loadedImageKey{resourceID: resourceID, width: uint32(cx), height: uint32(cy)}, nil
}

func (t *winTray) iconToBitmap(hIcon windows.Handle) (windows.Handle, error) {
	const SM_CXSMICON = 49
	const SM_CYSMICON = 50
	const DI_NORMAL = 0x3
	hDC, _, err := pGetDC.Call(uintptr(0))
	if hDC == 0 {
		return 0, err
	}
	defer pReleaseDC.Call(uintptr(0), hDC)
	hMemDC, _, err := pCreateCompatibleDC.Call(hDC)
	if hMemDC == 0 {
		return 0, err
	}
	defer pDeleteDC.Call(hMemDC)
	cx, _, _ := pGetSystemMetrics.Call(SM_CXSMICON)
	cy, _, _ := pGetSystemMetrics.Call(SM_CYSMICON)
	hMemBmp, _, err := pCreateCompatibleBitmap.Call(hDC, cx, cy)
	if hMemBmp == 0 {
		return 0, err
	}
	hOriginalBmp, _, _ := pSelectObject.Call(hMemDC, hMemBmp)
	defer pSelectObject.Call(hMemDC, hOriginalBmp)
	res, _, err := pDrawIconEx.Call(hMemDC, 0, 0, uintptr(hIcon), cx, cy, 0, uintptr(0), DI_NORMAL)
	if res == 0 {
		return 0, err
	}
	return windows.Handle(hMemBmp), nil
}
