//go:build ignore

// Command resourcegen generates architecture-specific Windows COFF resources.
package main

import (
	"bytes"
	"encoding/binary"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"strconv"
	"unicode/utf16"

	"github.com/akavel/rsrc/binutil"
	"github.com/akavel/rsrc/coff"
	"github.com/akavel/rsrc/ico"

	"github.com/JeffioZ/idletrigger/internal/platform/windows/resourceid"
)

const rtVersion = 16

type grpIconDir struct {
	ico.ICONDIR
	Entries []grpIconDirEntry
}

func (g grpIconDir) Size() int64 {
	return int64(binary.Size(g.ICONDIR) + len(g.Entries)*binary.Size(g.Entries[0]))
}

type grpIconDirEntry struct {
	ico.IconDirEntryCommon
	ID uint16
}

func main() {
	version := flag.String("version", "0.0.0", "product version string")
	manifest := flag.String("manifest", "build/windows/manifest.xml", "manifest path")
	icon := flag.String("ico", "build/windows/icons/app.ico", "icon path")
	trayDarkIcon := flag.String("tray-dark-ico", "build/windows/icons/tray-dark.ico", "dark tray icon path")
	trayLightIcon := flag.String("tray-light-ico", "build/windows/icons/tray-light.ico", "light tray icon path")
	flag.Parse()

	targets := []struct {
		arch, output, filename string
	}{
		{"386", "cmd/idletrigger/rsrc_windows_386.syso", "IdleTrigger-x86.exe"},
		{"amd64", "cmd/idletrigger/rsrc_windows_amd64.syso", "IdleTrigger-x64.exe"},
	}
	for _, target := range targets {
		if err := generate(target.arch, target.output, target.filename, *manifest, *icon, *trayDarkIcon, *trayLightIcon, *version); err != nil {
			fmt.Fprintf(os.Stderr, "%s: %v\n", target.output, err)
			os.Exit(1)
		}
		fmt.Printf("wrote %s\n", target.output)
	}
}

func generate(arch, output, originalFilename, manifestPath, iconPath, trayDarkIconPath, trayLightIconPath, version string) error {
	lastIconImageID := uint16(resourceid.TrayLightIconID)
	newIconImageID := func() uint16 {
		lastIconImageID++
		return lastIconImageID
	}

	out := coff.NewRSRC()
	if err := out.Arch(arch); err != nil {
		return err
	}

	manifest, err := binutil.SizedOpen(manifestPath)
	if err != nil {
		return fmt.Errorf("open manifest: %w", err)
	}
	defer manifest.Close()
	out.AddResource(coff.RT_MANIFEST, resourceid.ManifestID, manifest)

	iconFile, err := addIcon(out, iconPath, resourceid.AppIconID, newIconImageID)
	if err != nil {
		return err
	}
	defer iconFile.Close()
	trayDarkIconFile, err := addIcon(out, trayDarkIconPath, resourceid.TrayDarkIconID, newIconImageID)
	if err != nil {
		return err
	}
	defer trayDarkIconFile.Close()
	trayLightIconFile, err := addIcon(out, trayLightIconPath, resourceid.TrayLightIconID, newIconImageID)
	if err != nil {
		return err
	}
	defer trayLightIconFile.Close()

	out.AddResource(rtVersion, 1, bytes.NewReader(versionInfo(version, originalFilename)))
	out.Freeze()
	return writeCOFF(out, output)
}

func addIcon(out *coff.Coff, path string, groupID uint16, newIconImageID func() uint16) (io.Closer, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	icons, err := ico.DecodeHeaders(f)
	if err != nil {
		f.Close()
		return nil, err
	}
	group := grpIconDir{ICONDIR: ico.ICONDIR{Reserved: 0, Type: 1, Count: uint16(len(icons))}}
	for _, icon := range icons {
		id := newIconImageID()
		out.AddResource(coff.RT_ICON, id, io.NewSectionReader(f, int64(icon.ImageOffset), int64(icon.BytesInRes)))
		group.Entries = append(group.Entries, grpIconDirEntry{IconDirEntryCommon: icon.IconDirEntryCommon, ID: id})
	}
	out.AddResource(coff.RT_GROUP_ICON, groupID, group)
	return f, nil
}

func versionInfo(version, originalFilename string) []byte {
	fileVersion := numericVersion(version)
	fields := map[string]string{
		"CompanyName":      "JeffioZ",
		"FileDescription":  "IdleTrigger",
		"FileVersion":      fileVersion,
		"InternalName":     "IdleTrigger",
		"LegalCopyright":   "Copyright (C) 2026 JeffioZ",
		"OriginalFilename": originalFilename,
		"ProductName":      "IdleTrigger",
		"ProductVersion":   version,
	}
	var b blockBuilder
	root := b.begin("VS_VERSION_INFO", 0, 52)
	writeFixedFileInfo(&b, fileVersion)
	sfi := b.begin("StringFileInfo", 1, 0)
	table := b.begin("040904B0", 1, 0)
	keys := []string{"CompanyName", "FileDescription", "FileVersion", "InternalName", "LegalCopyright", "OriginalFilename", "ProductName", "ProductVersion"}
	for _, key := range keys {
		value := fields[key]
		child := b.begin(key, 1, uint16(len(utf16.Encode([]rune(value)))+1))
		b.writeUTF16Z(value)
		b.end(child)
	}
	b.end(table)
	b.end(sfi)
	vfi := b.begin("VarFileInfo", 1, 0)
	translation := b.begin("Translation", 0, 4)
	b.writeU16(0x0409)
	b.writeU16(0x04B0)
	b.end(translation)
	b.end(vfi)
	b.end(root)
	return b.Bytes()
}

type blockBuilder struct {
	bytes.Buffer
}

func (b *blockBuilder) begin(key string, typ, valueLength uint16) int {
	start := b.Len()
	b.writeU16(0)
	b.writeU16(valueLength)
	b.writeU16(typ)
	b.writeUTF16Z(key)
	b.align4()
	return start
}

func (b *blockBuilder) end(start int) {
	b.align4()
	binary.LittleEndian.PutUint16(b.Bytes()[start:start+2], uint16(b.Len()-start))
}

func (b *blockBuilder) writeU16(v uint16) { _ = binary.Write(b, binary.LittleEndian, v) }
func (b *blockBuilder) writeU32(v uint32) { _ = binary.Write(b, binary.LittleEndian, v) }
func (b *blockBuilder) writeUTF16Z(s string) {
	for _, c := range utf16.Encode([]rune(s)) {
		b.writeU16(c)
	}
	b.writeU16(0)
}
func (b *blockBuilder) align4() {
	for b.Len()%4 != 0 {
		b.WriteByte(0)
	}
}

func writeFixedFileInfo(b *blockBuilder, version string) {
	parts := versionParts(version)
	ms := uint32(parts[0])<<16 | uint32(parts[1])
	ls := uint32(parts[2])<<16 | uint32(parts[3])
	b.writeU32(0xFEEF04BD)
	b.writeU32(0x00010000)
	b.writeU32(ms)
	b.writeU32(ls)
	b.writeU32(ms)
	b.writeU32(ls)
	b.writeU32(0x0000003F)
	b.writeU32(0)
	b.writeU32(0x00040004)
	b.writeU32(0x00000001)
	b.writeU32(0)
	b.writeU32(0)
	b.writeU32(0)
}

func numericVersion(version string) string {
	parts := versionParts(version)
	return fmt.Sprintf("%d.%d.%d.%d", parts[0], parts[1], parts[2], parts[3])
}

func versionParts(version string) [4]int {
	re := regexp.MustCompile(`\d+`)
	matches := re.FindAllString(version, 4)
	var parts [4]int
	for i, match := range matches {
		v, _ := strconv.Atoi(match)
		parts[i] = v
	}
	return parts
}

func writeCOFF(coffFile *coff.Coff, output string) error {
	if err := os.MkdirAll(filepath.Dir(filepath.Clean(output)), 0o755); err != nil && filepath.Dir(filepath.Clean(output)) != "." {
		return err
	}
	out, err := os.Create(output)
	if err != nil {
		return err
	}
	defer out.Close()
	writer := binutil.Writer{W: out}
	binutil.Walk(coffFile, func(v reflect.Value, _ string) error {
		if binutil.Plain(v.Kind()) {
			writer.WriteLE(v.Interface())
			return nil
		}
		if sized, ok := v.Interface().(binutil.SizedReader); ok {
			writer.WriteFromSized(sized)
			return binutil.WALK_SKIP
		}
		return nil
	})
	if writer.Err != nil {
		return fmt.Errorf("write output: %w", writer.Err)
	}
	return nil
}
