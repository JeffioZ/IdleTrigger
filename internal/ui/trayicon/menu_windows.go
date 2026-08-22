package trayicon

import (
	"fmt"
	"sort"
	"unsafe"

	"github.com/JeffioZ/idletrigger/internal/feature/theme"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/darkmode"
	"golang.org/x/sys/windows"
)

func (t *winTray) createMenu() error {
	const (
		MIM_STYLE   = 0x00000010
		MNS_NOCHECK = 0x80000000 // No checkbox column: this menu contains plain commands only.
	)

	menuHandle, _, err := pCreatePopupMenu.Call()
	if menuHandle == 0 {
		return err
	}
	t.menu = windows.Handle(menuHandle)

	// https://msdn.microsoft.com/en-us/library/windows/desktop/ms647575(v=vs.85).aspx
	mi := struct {
		Size, Mask, Style, Max uint32
		Background             windows.Handle
		ContextHelpID          uint32
		MenuData               uintptr
	}{
		Mask:  MIM_STYLE,
		Style: MNS_NOCHECK,
	}
	mi.Size = uint32(unsafe.Sizeof(mi))

	res, _, err := pSetMenuInfo.Call(
		uintptr(t.menu),
		uintptr(unsafe.Pointer(&mi)),
	)
	if res == 0 {
		return err
	}
	return nil
}

func (t *winTray) addOrUpdateMenuItem(menuItemID uint32, title string) error {
	// https://msdn.microsoft.com/en-us/library/windows/desktop/ms647578(v=vs.85).aspx
	const (
		MIIM_FTYPE  = 0x00000100
		MIIM_STRING = 0x00000040
		MIIM_ID     = 0x00000002
	)
	const MFT_STRING = 0x00000000
	titlePtr, err := windows.UTF16PtrFromString(title)
	if err != nil {
		return err
	}

	mi := menuItemInfo{
		Mask:     MIIM_FTYPE | MIIM_STRING | MIIM_ID,
		Type:     MFT_STRING,
		ID:       menuItemID,
		TypeData: titlePtr,
		Cch:      uint32(len(title)),
	}
	mi.Size = uint32(unsafe.Sizeof(mi))
	var res uintptr
	if t.getVisibleItemIndex(menuItemID) != -1 {
		// We set the menu item info based on the menuID
		res, _, _ = pSetMenuItemInfo.Call(
			uintptr(t.menu),
			uintptr(menuItemID),
			0,
			uintptr(unsafe.Pointer(&mi)),
		)
	}

	if res == 0 {
		// Menu item does not already exist, create it
		t.addToVisibleItems(menuItemID)
		position := t.getVisibleItemIndex(menuItemID)
		res, _, err = pInsertMenuItem.Call(
			uintptr(t.menu),
			uintptr(position),
			1,
			uintptr(unsafe.Pointer(&mi)),
		)
		if res == 0 {
			t.delFromVisibleItems(menuItemID)
			return err
		}
	}

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
	darkmode.PreparePopupMenu(uintptr(window), theme.Current() == theme.ModeDark)

	defer pPostMessage.Call(uintptr(window), WM_NULL, 0, 0)
	res, _, err = pTrackPopupMenu.Call(
		uintptr(t.menu),
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

func (t *winTray) delFromVisibleItems(value uint32) {
	t.muVisibleItems.Lock()
	defer t.muVisibleItems.Unlock()
	for i, itemID := range t.visibleItems {
		if value == itemID {
			t.visibleItems = append(t.visibleItems[:i], t.visibleItems[i+1:]...)
			break
		}
	}
}

func (t *winTray) addToVisibleItems(value uint32) {
	t.muVisibleItems.Lock()
	defer t.muVisibleItems.Unlock()
	for _, itemID := range t.visibleItems {
		if itemID == value {
			return
		}
	}
	t.visibleItems = append(t.visibleItems, value)
	sort.Slice(t.visibleItems, func(i, j int) bool { return t.visibleItems[i] < t.visibleItems[j] })
}

func (t *winTray) getVisibleItemIndex(value uint32) int {
	t.muVisibleItems.RLock()
	defer t.muVisibleItems.RUnlock()
	for i, itemID := range t.visibleItems {
		if value == itemID {
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
