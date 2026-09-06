package nativeform

import (
	"testing"

	"golang.org/x/sys/windows"
)

func TestChoicePopupFitsLargeTextToSmallWorkArea(t *testing.T) {
	// Six 150% DPI / 225% text rows need 733 px, beyond this work area.
	const height, row, gap, inset = int32(700), int32(115), int32(3), int32(14)
	visible := choicePopupVisibleRows(6, height, row, gap, inset)
	if visible != 5 {
		t.Fatalf("visible rows = %d, want 5", visible)
	}
	p := &ChoicePopup{options: ChoicePopupOptions{MaxVisible: visible, Items: make([]ChoicePopupItem, 6)}}
	p.ensureVisible(5, p.visibleRows())
	if p.first != 1 {
		t.Fatal("last option did not scroll into the reduced viewport")
	}
	popupHeight := 2*inset + int32(visible)*row + int32(visible-1)*gap
	work := popupRect{Left: -1280, Top: -700, Right: 0, Bottom: 0}
	y := choicePopupVerticalPosition(popupRect{Top: -300, Bottom: -250}, work, popupHeight, 8, false)
	if y < work.Top || y+popupHeight > work.Bottom {
		t.Fatalf("popup outside work area: y=%d height=%d", y, popupHeight)
	}
}

func TestChoicePopupRowHitTestingSkipsHeadersAndGaps(t *testing.T) {
	p := &ChoicePopup{
		options: ChoicePopupOptions{
			MaxVisible: 3,
			Items: []ChoicePopupItem{
				{Label: "State", Header: true},
				{Label: "Enable", Value: 1},
				{Label: "Disable", Value: 2},
			},
		},
		rowHeight: 34,
		rowGap:    1,
		inset:     4,
	}
	if got := p.rowAt(4 + 17); got != -1 {
		t.Fatalf("header hit = %d, want -1", got)
	}
	if got := p.rowAt(4 + 34); got != -1 {
		t.Fatalf("row gap hit = %d, want -1", got)
	}
	if got := p.rowAt(4 + 35 + 17); got != 1 {
		t.Fatalf("option hit = %d, want 1", got)
	}
}

func TestChoicePopupKeyboardNavigationAndViewport(t *testing.T) {
	p := &ChoicePopup{
		options: ChoicePopupOptions{
			MaxVisible: 3,
			Items: []ChoicePopupItem{
				{Header: true}, {Value: 0}, {Value: 1},
				{Header: true}, {Value: 2}, {Value: 3},
			},
		},
	}
	if got := p.nextSelectable(2, 1); got != 4 {
		t.Fatalf("next selectable = %d, want 4", got)
	}
	p.ensureVisible(5, p.visibleRows())
	if p.first != 3 {
		t.Fatalf("viewport first = %d, want 3", p.first)
	}
	p.ensureVisible(1, p.visibleRows())
	if p.first != 1 {
		t.Fatalf("viewport first after moving up = %d, want 1", p.first)
	}
}

func TestChoicePopupFocusTransferWithinOwnerWaitsForOwnerCommand(t *testing.T) {
	owner := windows.Handle(41)
	anchor := windows.Handle(42)
	if !choicePopupFocusStaysWithinOwner(owner, anchor, anchor) {
		t.Fatal("focus transfer to the choice anchor should keep the popup alive")
	}
	if !choicePopupFocusStaysWithinOwner(owner, anchor, owner) {
		t.Fatal("focus transfer through the owner should keep the popup alive")
	}
	if choicePopupFocusStaysWithinOwner(owner, anchor, 0) {
		t.Fatal("focus leaving the owner should close the popup")
	}
}

func TestChoicePopupReselectionPolicyIsExplicit(t *testing.T) {
	if !choicePopupKeepsReselectionOpen(2, 2, true) {
		t.Fatal("configured reselection should remain open")
	}
	if choicePopupKeepsReselectionOpen(2, 2, false) {
		t.Fatal("default reselection should close")
	}
	if choicePopupKeepsReselectionOpen(2, 3, true) {
		t.Fatal("a different selection must be applied")
	}
}

func TestChoicePopupHomeAndEndSkipHeaders(t *testing.T) {
	p := &ChoicePopup{options: ChoicePopupOptions{Items: []ChoicePopupItem{
		{Label: "State", Header: true},
		{Label: "On", Value: 0},
		{Label: "System", Header: true},
		{Label: "Shutdown", Value: 1},
	}}}
	if got := p.nextSelectable(-1, 1); got != 1 {
		t.Fatalf("first selectable row = %d, want 1", got)
	}
	if got := p.nextSelectable(len(p.options.Items), -1); got != 3 {
		t.Fatalf("last selectable row = %d, want 3", got)
	}
}

func TestChoicePopupCanPreferAboveItsAnchor(t *testing.T) {
	work := popupRect{Top: 0, Bottom: 900}
	anchor := popupRect{Top: 700, Bottom: 740}
	if got := choicePopupVerticalPosition(anchor, work, 180, 4, true); got != 516 {
		t.Fatalf("preferred-above y = %d, want 516", got)
	}
	if got := choicePopupVerticalPosition(anchor, work, 100, 4, false); got != 744 {
		t.Fatalf("default-below y = %d, want 744", got)
	}
}

func TestChoicePopupPreferredSideFallsBackWhenItDoesNotFit(t *testing.T) {
	work := popupRect{Top: 100, Bottom: 700}
	anchor := popupRect{Top: 150, Bottom: 190}
	if got := choicePopupVerticalPosition(anchor, work, 240, 4, true); got != 194 {
		t.Fatalf("above fallback y = %d, want 194", got)
	}
	anchor = popupRect{Top: 610, Bottom: 650}
	if got := choicePopupVerticalPosition(anchor, work, 240, 4, false); got != 366 {
		t.Fatalf("below fallback y = %d, want 366", got)
	}
}
