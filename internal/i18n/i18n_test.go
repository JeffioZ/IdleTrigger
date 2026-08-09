package i18n

import (
	"go/ast"
	"go/parser"
	"go/token"
	"io/fs"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/JeffioZ/idletrigger/internal/version"
)

func TestLocalesHaveMatchingKeys(t *testing.T) {
	en := store["en"]
	zh := store["zh-CN"]
	if len(en) == 0 || len(zh) == 0 {
		t.Fatal("embedded locales were not loaded")
	}
	for key := range en {
		if _, ok := zh[key]; !ok {
			t.Errorf("zh-CN locale is missing key %q", key)
		}
	}
	for key := range zh {
		if _, ok := en[key]; !ok {
			t.Errorf("English locale is missing key %q", key)
		}
	}
}

func TestLocalesUseMatchingFormatVerbs(t *testing.T) {
	verbPattern := regexp.MustCompile(`%[-+# 0-9.]*[A-Za-z%]`)
	for key, enText := range store["en"] {
		zhText := store["zh-CN"][key]
		enVerbs := strings.Join(verbPattern.FindAllString(enText, -1), ",")
		zhVerbs := strings.Join(verbPattern.FindAllString(zhText, -1), ",")
		if enVerbs != zhVerbs {
			t.Errorf("format verbs differ for %q: en=%q zh-CN=%q", key, enVerbs, zhVerbs)
		}
	}
}

func TestTooltipCopyHasNoTrailingSentencePunctuation(t *testing.T) {
	for language, values := range store {
		for key, value := range values {
			if !strings.HasPrefix(key, "tip_") {
				continue
			}
			value = strings.TrimSpace(value)
			if strings.HasSuffix(value, ".") || strings.HasSuffix(value, "。") {
				t.Errorf("%s tooltip %q ends with sentence punctuation: %q", language, key, value)
			}
		}
	}
}

func TestLiteralLocaleKeysUsedBySourceExist(t *testing.T) {
	missing := map[string][]string{}
	root := filepath.Join("..", "..")
	for _, sourceRoot := range []string{"cmd", "internal"} {
		err := filepath.WalkDir(filepath.Join(root, sourceRoot), func(path string, entry fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if entry.IsDir() || filepath.Ext(path) != ".go" || strings.HasSuffix(path, "_test.go") {
				return nil
			}
			file, err := parser.ParseFile(token.NewFileSet(), path, nil, 0)
			if err != nil {
				return err
			}
			ast.Inspect(file, func(node ast.Node) bool {
				call, ok := node.(*ast.CallExpr)
				if !ok {
					return true
				}
				key, ok := literalLocaleKey(call)
				if !ok {
					return true
				}
				if _, exists := store["en"][key]; !exists {
					relative, relErr := filepath.Rel(root, path)
					if relErr != nil {
						relative = path
					}
					missing[key] = append(missing[key], relative)
				}
				return true
			})
			return nil
		})
		if err != nil {
			t.Fatalf("scan %s: %v", sourceRoot, err)
		}
	}

	keys := make([]string, 0, len(missing))
	for key := range missing {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	for _, key := range keys {
		t.Errorf("source uses missing locale key %q in %s", key, strings.Join(missing[key], ", "))
	}
}

func literalLocaleKey(call *ast.CallExpr) (string, bool) {
	argument := -1
	switch function := call.Fun.(type) {
	case *ast.Ident:
		if function.Name == "T" {
			argument = 1
		}
	case *ast.SelectorExpr:
		switch function.Sel.Name {
		case "T":
			argument = 1
		case "text", "t":
			argument = 0
		}
	}
	if argument < 0 || len(call.Args) <= argument {
		return "", false
	}
	literal, ok := call.Args[argument].(*ast.BasicLit)
	if !ok || literal.Kind != token.STRING {
		return "", false
	}
	key, err := strconv.Unquote(literal.Value)
	return key, err == nil
}

func TestLiteralLocaleKeyRecognizesPanelTranslator(t *testing.T) {
	expression, err := parser.ParseExpr(`panel.t("automation_title")`)
	if err != nil {
		t.Fatal(err)
	}
	call, ok := expression.(*ast.CallExpr)
	if !ok {
		t.Fatalf("expression type = %T", expression)
	}
	if key, ok := literalLocaleKey(call); !ok || key != "automation_title" {
		t.Fatalf("literalLocaleKey() = %q, %v", key, ok)
	}
}

func TestVersionPlaceholder(t *testing.T) {
	original := version.Value
	version.Value = "v-test"
	t.Cleanup(func() { version.Value = original })

	got := T("en", "version")
	if !strings.Contains(got, "v-test") || strings.Contains(got, "{{version}}") {
		t.Fatalf("version placeholder was not expanded: %q", got)
	}
}

func TestResolveLanguageKeepsExplicitChoice(t *testing.T) {
	if got := ResolveLanguage("en"); got != "en" {
		t.Fatalf("ResolveLanguage(en) = %q", got)
	}
	if got := ResolveLanguage("zh-CN"); got != "zh-CN" {
		t.Fatalf("ResolveLanguage(zh-CN) = %q", got)
	}
}

func TestFormatDuration(t *testing.T) {
	d := time.Hour + 2*time.Minute + 3*time.Second
	if got := FormatDuration("en", d); got != "1h 2m 3s" {
		t.Fatalf("English duration = %q", got)
	}
	if got := FormatDuration("zh-CN", d); got != "1 小时 2 分钟 3 秒" {
		t.Fatalf("Chinese duration = %q", got)
	}
}
