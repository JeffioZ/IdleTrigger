package processpicker

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode/utf16"
	"unsafe"

	"golang.org/x/sys/windows"

	"github.com/JeffioZ/idletrigger/internal/automation"
	"github.com/JeffioZ/idletrigger/internal/platform/windows/processcatalog"
	"github.com/JeffioZ/idletrigger/internal/ui/nativeform"
)

func (p *picker) startLoad(mode processLoadMode) {
	if p.closed.Load() || p.hwnd == 0 {
		return
	}
	if mode == processLoadAutomatic && p.loadInFlight {
		return
	}
	if mode != processLoadAutomatic {
		p.captureSelection()
	}
	p.captureViewAnchors()
	p.loadMode = mode
	p.loadInFlight = true
	if mode == processLoadManual {
		p.beginManualStatusHold()
	} else {
		p.clearStatusHold()
	}
	if mode != processLoadAutomatic {
		p.transitionView(pickerViewLoading, "")
	}
	generation := nextGeneration.Add(1)
	p.generation = generation
	var exactTargets []automation.ProcessTarget
	for _, target := range p.selected {
		if target.Match == automation.MatchPath {
			exactTargets = append(exactTargets, target)
		}
	}
	cachedDescriptions := make(map[string]string, len(p.descriptions))
	for key, value := range p.descriptions {
		cachedDescriptions[key] = value
	}
	attemptedDescriptions := make(map[string]struct{}, len(p.descriptionTried))
	for key := range p.descriptionTried {
		attemptedDescriptions[key] = struct{}{}
	}
	p.workers.Add(1)
	go func() {
		defer p.workers.Done()
		scanStarted := time.Now()
		instances, err := snapshotNamesForPicker()
		scanDuration := time.Since(scanStarted)
		if err != nil {
			postPickerUI(func() { p.finishLoad(generation, nil, err, true, scanDuration) })
			return
		}
		instances = excludePickerProcess(instances, uint32(os.Getpid()))
		for index := range instances {
			key := (automation.ProcessTarget{Match: automation.MatchName, Executable: instances[index].Executable}).Key()
			instances[index].Description = cachedDescriptions[key]
		}
		candidates, nameAttempts := descriptionCandidates(instances, cachedDescriptions, attemptedDescriptions, mode == processLoadManual)
		exactAttempts := make([]automation.ProcessTarget, 0, len(exactTargets))
		for _, target := range exactTargets {
			key := target.Key()
			if cachedDescriptions[key] != "" {
				continue
			}
			if _, tried := attemptedDescriptions[key]; tried && mode != processLoadManual {
				continue
			}
			exactAttempts = append(exactAttempts, target)
		}
		hasMetadataWork := len(nameAttempts) > 0 || len(exactAttempts) > 0
		postPickerUI(func() { p.finishLoad(generation, instances, nil, !hasMetadataWork, scanDuration) })
		nameDescriptions := make(map[string]string, len(nameAttempts))
		if len(candidates) > 0 {
			for _, instance := range enrichDescriptionsForPicker(candidates) {
				if instance.Description == "" {
					continue
				}
				key := (automation.ProcessTarget{Match: automation.MatchName, Executable: instance.Executable}).Key()
				nameDescriptions[key] = instance.Description
			}
		}
		exactDescriptions := make(map[string]string, len(exactAttempts))
		for _, target := range exactAttempts {
			if description := fileDescriptionForPicker(target.Path); description != "" {
				exactDescriptions[target.Key()] = description
			}
		}
		if hasMetadataWork {
			postPickerUI(func() {
				p.finishDescriptionLoad(generation, instances, nameAttempts, nameDescriptions, exactAttempts, exactDescriptions)
			})
		}
	}()
}

func excludePickerProcess(instances []processcatalog.Instance, processID uint32) []processcatalog.Instance {
	if processID == 0 {
		return instances
	}
	filtered := instances[:0]
	for _, instance := range instances {
		if instance.PID != processID {
			filtered = append(filtered, instance)
		}
	}
	return filtered
}

func descriptionCandidates(instances []processcatalog.Instance, cached map[string]string, attempted map[string]struct{}, retry bool) ([]processcatalog.Instance, []string) {
	wanted := make(map[string]struct{})
	attempts := make([]string, 0)
	for _, instance := range instances {
		key := (automation.ProcessTarget{Match: automation.MatchName, Executable: instance.Executable}).Key()
		if cached[key] != "" {
			continue
		}
		if _, tried := attempted[key]; tried && !retry {
			continue
		}
		if _, exists := wanted[key]; !exists {
			wanted[key] = struct{}{}
			attempts = append(attempts, key)
		}
	}
	candidates := make([]processcatalog.Instance, 0, len(instances))
	for _, instance := range instances {
		key := (automation.ProcessTarget{Match: automation.MatchName, Executable: instance.Executable}).Key()
		if _, ok := wanted[key]; ok {
			candidates = append(candidates, instance)
		}
	}
	return candidates, attempts
}

func (p *picker) finishDescriptionLoad(generation uint64, instances []processcatalog.Instance, nameAttempts []string, nameDescriptions map[string]string, exactAttempts []automation.ProcessTarget, exactDescriptions map[string]string) {
	if p.closed.Load() || p.hwnd == 0 || p.generation != generation {
		return
	}
	for _, key := range nameAttempts {
		p.descriptionTried[key] = struct{}{}
	}
	for _, target := range exactAttempts {
		p.descriptionTried[target.Key()] = struct{}{}
	}
	for key, value := range nameDescriptions {
		p.descriptions[key] = value
	}
	for key, value := range exactDescriptions {
		p.descriptions[key] = value
	}
	for index := range instances {
		key := (automation.ProcessTarget{Match: automation.MatchName, Executable: instances[index].Executable}).Key()
		instances[index].Description = p.descriptions[key]
	}
	p.finishLoad(generation, instances, nil, true, 0)
}

func (p *picker) finishLoad(generation uint64, instances []processcatalog.Instance, err error, final bool, scanDuration time.Duration) {
	if p.closed.Load() || p.hwnd == 0 || p.generation != generation {
		return
	}
	if err != nil {
		p.loadInFlight = false
		if p.loadMode == processLoadAutomatic {
			p.anchorTopKey, p.anchorFocusKey = "", ""
			return
		}
		p.transitionView(pickerViewError, fmt.Sprintf(p.text("process_picker_error"), err.Error()))
		p.anchorTopKey, p.anchorFocusKey = "", ""
		p.repaint()
		return
	}
	if scanDuration > 0 {
		p.lastSnapshot = time.Now()
		p.lastScanDuration = scanDuration
	}
	groups := processcatalog.GroupInstances(instances)
	for index := range groups {
		key := (automation.ProcessTarget{Match: automation.MatchName, Executable: groups[index].Executable}).Key()
		if groups[index].Description != "" {
			p.descriptions[key] = groups[index].Description
		} else if cached := p.descriptions[key]; cached != "" {
			groups[index].Description = cached
		}
	}
	nextItems := buildItems(groups, p.selected, p.options.Text)
	unchangedSilentRefresh := p.loadMode == processLoadAutomatic && samePickerItems(p.items, nextItems)
	p.items = nextItems
	phase := pickerViewReady
	if !final && p.loadMode != processLoadAutomatic {
		phase = pickerViewEnriching
	}
	if final {
		p.loadInFlight = false
	}
	if unchangedSilentRefresh {
		p.viewPhase = phase
		if final {
			p.anchorTopKey, p.anchorFocusKey = "", ""
		}
		return
	}
	p.transitionView(phase, "")
	p.applyFilter()
	p.restoreViewAnchors(final)
	p.updatePreview()
	// The snapshot, header, rows, status, preview and themed backing surfaces
	// form one visible state transition. Commit that state as a complete frame;
	// per-control updates remain useful for search and checkbox interaction, but
	// are not sufficient for a newly completed asynchronous load on all builds.
	p.repaint()
}

func (p *picker) transitionView(phase pickerViewPhase, detail string) {
	p.viewPhase = phase
	p.enable(idConfirm, len(p.selected) > 0 && len(p.selected) <= automation.MaxProcessesPerRule)
	switch phase {
	case pickerViewLoading:
		p.setText(idStatus, p.text("process_picker_loading"))
		p.setText(idEmpty, p.text("process_picker_loading"))
		p.show(idEmpty, len(p.visible) == 0)
		p.enable(idRefresh, false)
	case pickerViewError:
		p.clearStatusHold()
		p.setText(idStatus, detail)
		p.setText(idEmpty, p.text("process_picker_load_failed"))
		p.show(idEmpty, len(p.visible) == 0)
		p.enable(idRefresh, true)
	default:
		p.enable(idRefresh, true)
	}
}

func (p *picker) refreshViewStatus() {
	switch p.viewPhase {
	case pickerViewLoading, pickerViewError:
		return
	case pickerViewEnriching:
		p.setStatus(p.text("process_picker_loading_descriptions"))
	default:
		p.updateSelectionStatus()
	}
}

func (p *picker) beginManualStatusHold() {
	p.clearStatusHold()
	p.statusHoldUntil = time.Now().Add(processPickerManualStatusHold)
	if timer, _, _ := pSetTimer.Call(uintptr(p.hwnd), statusTimerID, uintptr(processPickerManualStatusHold/time.Millisecond), 0); timer == 0 {
		p.statusHoldUntil = time.Time{}
	}
}

func (p *picker) clearStatusHold() {
	if p.hwnd != 0 {
		pKillTimer.Call(uintptr(p.hwnd), statusTimerID)
	}
	p.statusHoldUntil = time.Time{}
	p.pendingStatus = ""
}

func (p *picker) setStatus(value string) {
	if delay := processPickerStatusHoldDelay(time.Now(), p.statusHoldUntil); delay > 0 {
		p.pendingStatus = value
		return
	}
	p.clearStatusHold()
	p.setText(idStatus, value)
}

func (p *picker) finishStatusHold() {
	if delay := processPickerStatusHoldDelay(time.Now(), p.statusHoldUntil); delay > 0 {
		milliseconds := (delay + time.Millisecond - 1) / time.Millisecond
		if timer, _, _ := pSetTimer.Call(uintptr(p.hwnd), statusTimerID, uintptr(milliseconds), 0); timer != 0 {
			return
		}
	}
	pending := p.pendingStatus
	p.clearStatusHold()
	if pending != "" {
		p.setText(idStatus, pending)
		return
	}
	if p.viewPhase == pickerViewLoading {
		// A slow snapshot remains a loading operation after the minimum display
		// interval. Its completion will publish the result count normally.
		return
	}
	p.refreshViewStatus()
}

func processPickerStatusHoldDelay(now, until time.Time) time.Duration {
	if until.IsZero() || !now.Before(until) {
		return 0
	}
	return until.Sub(now)
}

func (p *picker) captureViewAnchors() {
	list := p.controls[idList]
	if list == 0 || len(p.visible) == 0 {
		p.anchorTopKey, p.anchorFocusKey = "", ""
		return
	}
	top, _, _ := pSendMessage.Call(uintptr(list), lvmGetTopIndex, 0, 0)
	if int(top) >= 0 && int(top) < len(p.visible) {
		p.anchorTopKey = p.visible[int(top)].target.Key()
	}
	p.anchorFocusKey = ""
	for index, value := range p.visible {
		state, _, _ := pSendMessage.Call(uintptr(list), lvmGetItemState, uintptr(index), lvisFocused)
		if state&lvisFocused != 0 {
			p.anchorFocusKey = value.target.Key()
			break
		}
	}
}

func (p *picker) restoreViewAnchors(clear bool) {
	indexOf := func(key string) int {
		for index, value := range p.visible {
			if value.target.Key() == key {
				return index
			}
		}
		return -1
	}
	if index := indexOf(p.anchorTopKey); index >= 0 {
		p.scrollListTo(index)
	}
	if index := indexOf(p.anchorFocusKey); index >= 0 {
		entry := lvItem{StateMask: lvisFocused | lvisSelected, State: lvisFocused | lvisSelected}
		pSendMessage.Call(uintptr(p.controls[idList]), lvmSetItemState, uintptr(index), uintptr(unsafe.Pointer(&entry)))
		pSendMessage.Call(uintptr(p.controls[idList]), lvmEnsureVisible, uintptr(index), 0)
	}
	if clear {
		p.anchorTopKey, p.anchorFocusKey = "", ""
	}
}

func processPickerRefreshAge(scanDuration time.Duration) time.Duration {
	age := scanDuration * processPickerScanCostMultiplier
	if age < processPickerAutoRefreshMinAge {
		return processPickerAutoRefreshMinAge
	}
	if age > processPickerAutoRefreshMaxAge {
		return processPickerAutoRefreshMaxAge
	}
	return age
}

func shouldAutoRefreshProcessPicker(last, now time.Time, scanDuration time.Duration, loading bool) bool {
	return !loading && !last.IsZero() && now.Sub(last) >= processPickerRefreshAge(scanDuration)
}

func samePickerItems(left, right []item) bool {
	if len(left) != len(right) {
		return false
	}
	for index := range left {
		if left[index].target.Key() != right[index].target.Key() ||
			left[index].name != right[index].name ||
			left[index].description != right[index].description ||
			left[index].count != right[index].count ||
			left[index].search != right[index].search {
			return false
		}
	}
	return true
}

func buildItems(groups []processcatalog.Group, selected map[string]automation.ProcessTarget, text func(string) string) []item {
	items := make([]item, 0, len(groups)+len(selected))
	present := make(map[string]struct{})
	sort.SliceStable(groups, func(i, j int) bool {
		return strings.ToLower(groups[i].Executable) < strings.ToLower(groups[j].Executable)
	})
	for _, group := range groups {
		target := automation.ProcessTarget{Match: automation.MatchName, Executable: group.Executable}
		items = append(items, item{
			target: target, name: group.Executable, description: group.Description,
			count:  strconv.Itoa(group.Count),
			search: strings.ToLower(group.Executable + " " + group.Description),
		})
		present[target.Key()] = struct{}{}
	}
	missingSelected := make([]automation.ProcessTarget, 0, len(selected))
	for key, target := range selected {
		if target.Match == automation.MatchPath {
			continue
		}
		if _, ok := present[key]; ok {
			continue
		}
		missingSelected = append(missingSelected, target)
	}
	sort.Slice(missingSelected, func(i, j int) bool {
		leftName := strings.ToLower(missingSelected[i].Executable)
		rightName := strings.ToLower(missingSelected[j].Executable)
		if leftName != rightName {
			return leftName < rightName
		}
		return missingSelected[i].Key() < missingSelected[j].Key()
	})
	for _, target := range missingSelected {
		description := text("process_picker_not_running")
		items = append(items, item{target: target, name: target.Executable, description: description, search: strings.ToLower(target.Executable + " " + description)})
	}
	return items
}

func (p *picker) applyFilter() {
	filter := strings.ToLower(strings.TrimSpace(p.controlText(idSearch)))
	sorted := sortItems(p.items, p.sortColumn, p.sortAscending)
	next := filterItems(sorted, filter)
	previous := p.visible
	p.visible = next
	p.reconcileVisible(previous, next)
	if len(p.visible) == 0 {
		message := p.emptyMessage(filter)
		p.setText(idEmpty, message)
		p.show(idEmpty, true)
	} else {
		p.show(idEmpty, false)
	}
	p.refreshViewStatus()
}

func (p *picker) emptyMessage(filter string) string {
	switch p.viewPhase {
	case pickerViewLoading:
		return p.text("process_picker_loading")
	case pickerViewError:
		return p.text("process_picker_load_failed")
	}
	if filter != "" {
		return p.text("process_picker_no_results")
	}
	return p.text("process_picker_empty")
}

func (p *picker) reconcileVisible(previous, next []item) {
	list := p.controls[idList]
	if list == 0 {
		return
	}
	p.populating = true
	pSendMessage.Call(uintptr(list), wmSetRedraw, 0, 0)
	if len(next) == 0 || !sameRelativeTargetOrder(previous, next) {
		pSendMessage.Call(uintptr(list), lvmDeleteAllItems, 0, 0)
		for index, value := range next {
			p.insertItem(index, value)
		}
	} else {
		nextKeys := make(map[string]struct{}, len(next))
		for _, value := range next {
			nextKeys[value.target.Key()] = struct{}{}
		}
		retained := make([]item, 0, min(len(previous), len(next)))
		for index := len(previous) - 1; index >= 0; index-- {
			if _, keep := nextKeys[previous[index].target.Key()]; keep {
				continue
			}
			pSendMessage.Call(uintptr(list), lvmDeleteItem, uintptr(index), 0)
		}
		for _, value := range previous {
			if _, keep := nextKeys[value.target.Key()]; keep {
				retained = append(retained, value)
			}
		}
		retainedIndex := 0
		for index, value := range next {
			if retainedIndex < len(retained) && retained[retainedIndex].target.Key() == value.target.Key() {
				p.updateItem(index, retained[retainedIndex], value)
				retainedIndex++
				continue
			}
			p.insertItem(index, value)
		}
	}
	pSendMessage.Call(uintptr(list), wmSetRedraw, 1, 0)
	p.populating = false
	if len(next) > 0 {
		pSendMessage.Call(uintptr(list), lvmRedrawItems, 0, uintptr(len(next)-1))
	}
	p.syncListScrollbarBounds()
	p.syncListScrollbar()
	nativeform.PresentControl(list, true)
}

func sameRelativeTargetOrder(previous, next []item) bool {
	if len(previous) == 0 || len(next) == 0 {
		return true
	}
	nextKeys := make(map[string]struct{}, len(next))
	previousKeys := make(map[string]struct{}, len(previous))
	for _, value := range next {
		nextKeys[value.target.Key()] = struct{}{}
	}
	for _, value := range previous {
		previousKeys[value.target.Key()] = struct{}{}
	}
	commonPrevious := make([]string, 0, min(len(previous), len(next)))
	commonNext := make([]string, 0, cap(commonPrevious))
	for _, value := range previous {
		if _, ok := nextKeys[value.target.Key()]; ok {
			commonPrevious = append(commonPrevious, value.target.Key())
		}
	}
	for _, value := range next {
		if _, ok := previousKeys[value.target.Key()]; ok {
			commonNext = append(commonNext, value.target.Key())
		}
	}
	if len(commonPrevious) == 0 {
		return false
	}
	if len(commonPrevious) != len(commonNext) {
		return false
	}
	for index := range commonPrevious {
		if commonPrevious[index] != commonNext[index] {
			return false
		}
	}
	return true
}

func (p *picker) captureSelection() {
	list := p.controls[idList]
	if list == 0 {
		return
	}
	for index := range p.visible {
		state, _, _ := pSendMessage.Call(uintptr(list), lvmGetItemState, uintptr(index), lvisStateImageMask)
		key := p.visible[index].target.Key()
		if uint32(state)&lvisStateImageMask == 2<<12 {
			p.selected[key] = p.visible[index].target
		} else {
			delete(p.selected, key)
		}
	}
	p.selected = normalizeSelected(p.selected)
	p.syncCheckStates()
	p.updatePreview()
}

func (p *picker) insertItem(index int, value item) {
	columns := []string{value.name, value.description, value.count}
	text, _ := windows.UTF16PtrFromString(columns[0])
	entry := lvItem{Mask: lvifText, Item: int32(index), Text: text}
	pSendMessage.Call(uintptr(p.controls[idList]), lvmInsertItemW, 0, uintptr(unsafe.Pointer(&entry)))
	for column := 1; column < len(columns); column++ {
		value, _ := windows.UTF16PtrFromString(columns[column])
		entry = lvItem{SubItem: int32(column), Text: value}
		pSendMessage.Call(uintptr(p.controls[idList]), lvmSetItemTextW, uintptr(index), uintptr(unsafe.Pointer(&entry)))
	}
	checked := uint32(1 << 12)
	if _, ok := p.selected[value.target.Key()]; ok {
		checked = 2 << 12
	}
	entry = lvItem{StateMask: lvisStateImageMask, State: checked}
	pSendMessage.Call(uintptr(p.controls[idList]), lvmSetItemState, uintptr(index), uintptr(unsafe.Pointer(&entry)))
}

func (p *picker) updateItem(index int, previous, next item) {
	previousColumns := []string{previous.name, previous.description, previous.count}
	nextColumns := []string{next.name, next.description, next.count}
	for column, value := range nextColumns {
		if value == previousColumns[column] {
			continue
		}
		text, _ := windows.UTF16PtrFromString(value)
		entry := lvItem{SubItem: int32(column), Text: text}
		pSendMessage.Call(uintptr(p.controls[idList]), lvmSetItemTextW, uintptr(index), uintptr(unsafe.Pointer(&entry)))
	}
	state := uint32(1 << 12)
	if _, ok := p.selected[next.target.Key()]; ok {
		state = 2 << 12
	}
	entry := lvItem{StateMask: lvisStateImageMask, State: state}
	pSendMessage.Call(uintptr(p.controls[idList]), lvmSetItemState, uintptr(index), uintptr(unsafe.Pointer(&entry)))
}

func (p *picker) syncCheckStates() {
	if p.controls[idList] == 0 {
		return
	}
	p.populating = true
	defer func() { p.populating = false }()
	for index, value := range p.visible {
		state := uint32(1 << 12)
		if _, ok := p.selected[value.target.Key()]; ok {
			state = 2 << 12
		}
		entry := lvItem{StateMask: lvisStateImageMask, State: state}
		pSendMessage.Call(uintptr(p.controls[idList]), lvmSetItemState, uintptr(index), uintptr(unsafe.Pointer(&entry)))
	}
}

func (p *picker) updatePreview() {
	preview := p.controls[idPreview]
	if preview == 0 {
		return
	}
	targets := make([]automation.ProcessTarget, 0, len(p.selected))
	for _, target := range p.selected {
		targets = append(targets, target)
	}
	targets = automation.NormalizeTargets(targets)
	pSendMessage.Call(uintptr(preview), lbResetContent, 0, 0)
	for _, target := range targets {
		name := target.Executable
		if description := p.descriptions[target.Key()]; description != "" {
			name = fmt.Sprintf(p.text("process_name_description"), name, description)
		}
		label := fmt.Sprintf(p.text("process_picker_preview_name"), name)
		if target.Match == automation.MatchPath {
			label = fmt.Sprintf(p.text("process_picker_preview_path"), name, target.Path)
		}
		text, _ := windows.UTF16PtrFromString(label)
		pSendMessage.Call(uintptr(preview), lbAddString, 0, uintptr(unsafe.Pointer(text)))
	}
	if len(targets) == 0 {
		text, _ := windows.UTF16PtrFromString(p.text("process_picker_preview_empty"))
		pSendMessage.Call(uintptr(preview), lbAddString, 0, uintptr(unsafe.Pointer(text)))
	}
	p.setText(idPreviewTitle, fmt.Sprintf(p.text("process_picker_selection_title"), len(targets)))
	p.enable(idConfirm, len(targets) > 0 && len(targets) <= automation.MaxProcessesPerRule)
	if p.previewScroll != nil {
		p.previewScroll.Sync()
	}
	nativeform.PresentControl(preview, true)
}

func filterItems(values []item, filter string) []item {
	if filter == "" {
		return append([]item(nil), values...)
	}
	out := make([]item, 0, len(values))
	for _, value := range values {
		if strings.Contains(value.search, filter) {
			out = append(out, value)
		}
	}
	return out
}

func sortItems(values []item, column int, ascending bool) []item {
	out := append([]item(nil), values...)
	type sortKey struct {
		value item
		key   string
	}
	keys := make([]sortKey, len(out))
	for i, value := range out {
		switch column {
		case 1:
			keys[i] = sortKey{value, strings.ToLower(value.description)}
		case 2:
			count, _ := strconv.Atoi(value.count)
			keys[i] = sortKey{value, fmt.Sprintf("%08d", count)}
		default:
			keys[i] = sortKey{value, strings.ToLower(value.name)}
		}
	}
	sort.SliceStable(keys, func(i, j int) bool {
		if ascending {
			return keys[i].key < keys[j].key
		}
		return keys[i].key > keys[j].key
	})
	for i := range keys {
		out[i] = keys[i].value
	}
	return out
}

func normalizeSelected(values map[string]automation.ProcessTarget) map[string]automation.ProcessTarget {
	targets := make([]automation.ProcessTarget, 0, len(values))
	for _, target := range values {
		targets = append(targets, target)
	}
	normalized := automation.NormalizeTargets(targets)
	out := make(map[string]automation.ProcessTarget, len(normalized))
	for _, target := range normalized {
		out[target.Key()] = target
	}
	return out
}

func (p *picker) updateSelectionStatus() {
	if len(p.selected) > automation.MaxProcessesPerRule {
		p.setText(idStatus, fmt.Sprintf(p.text("process_picker_limit"), automation.MaxProcessesPerRule))
		return
	}
	p.setStatus(fmt.Sprintf(p.text("process_picker_status_results"), len(p.visible), len(p.selected)))
}

func canAddSelection(values map[string]automation.ProcessTarget, target automation.ProcessTarget) bool {
	if _, exists := values[target.Key()]; exists {
		return true
	}
	targets := make([]automation.ProcessTarget, 0, len(values)+1)
	for _, existing := range values {
		targets = append(targets, existing)
	}
	targets = append(targets, target)
	return len(automation.NormalizeTargets(targets)) <= automation.MaxProcessesPerRule
}

func (p *picker) confirm() {
	p.captureSelection()
	if len(p.selected) > automation.MaxProcessesPerRule {
		p.updateSelectionStatus()
		return
	}
	targets := make([]automation.ProcessTarget, 0, len(p.selected))
	for _, target := range p.selected {
		targets = append(targets, target)
	}
	targets = automation.NormalizeTargets(targets)
	descriptions := make(map[string]string, len(targets))
	for _, target := range targets {
		if description := p.descriptions[target.Key()]; description != "" {
			descriptions[target.Key()] = description
		}
	}
	callback := p.options.OnConfirm
	p.destroy()
	if callback != nil {
		callback(targets, descriptions)
	}
}

func (p *picker) browseExecutable() {
	filter := utf16.Encode([]rune(p.text("process_picker_exe_filter") + "\x00*.exe\x00\x00"))
	file := make([]uint16, 32768)
	title, _ := windows.UTF16PtrFromString(p.text("process_picker_browse_title"))
	defaultExtension, _ := windows.UTF16PtrFromString("exe")
	dialog := openFileName{
		Size: uint32(unsafe.Sizeof(openFileName{})), Owner: p.hwnd,
		Filter: &filter[0], FilterIndex: 1, File: &file[0], MaxFile: uint32(len(file)),
		Title: title, DefaultExtension: defaultExtension,
		Flags: ofnHideReadOnly | ofnNoChangeDir | ofnPathMustExist | ofnFileMustExist | ofnExplorer | ofnDontAddToRecent,
	}
	chosen, _, _ := pGetOpenFileName.Call(uintptr(unsafe.Pointer(&dialog)))
	if chosen == 0 {
		return
	}
	path := filepath.Clean(windows.UTF16ToString(file))
	pathPtr, err := windows.UTF16PtrFromString(path)
	if err != nil || !strings.EqualFold(filepath.Ext(path), ".exe") {
		p.setText(idStatus, p.text("process_picker_invalid_exe"))
		return
	}
	var binaryType uint32
	valid, _, _ := pGetBinaryType.Call(uintptr(unsafe.Pointer(pathPtr)), uintptr(unsafe.Pointer(&binaryType)))
	if valid == 0 {
		p.setText(idStatus, p.text("process_picker_invalid_exe"))
		return
	}
	target := automation.ProcessTarget{Match: automation.MatchPath, Executable: filepath.Base(path), Path: path}
	candidate := make(map[string]automation.ProcessTarget, len(p.selected)+1)
	for key, existing := range p.selected {
		candidate[key] = existing
	}
	for key, existing := range candidate {
		if existing.Match == automation.MatchName && strings.EqualFold(existing.Executable, target.Executable) {
			delete(candidate, key)
		}
	}
	if !canAddSelection(candidate, target) {
		p.setText(idStatus, fmt.Sprintf(p.text("process_picker_limit"), automation.MaxProcessesPerRule))
		return
	}
	candidate[target.Key()] = target
	p.selected = normalizeSelected(candidate)
	p.syncCheckStates()
	p.updatePreview()
	p.refreshViewStatus()
	key := target.Key()
	p.workers.Add(1)
	go func() {
		defer p.workers.Done()
		description := fileDescriptionForPicker(path)
		postPickerUI(func() {
			if p.closed.Load() || p.hwnd == 0 {
				return
			}
			p.descriptionTried[key] = struct{}{}
			if description != "" {
				p.descriptions[key] = description
			}
			p.updatePreview()
		})
	}()
}
