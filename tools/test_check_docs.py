#!/usr/bin/env python3
"""Regression tests; run with `uv run tools/check_docs.py --test`."""

from pathlib import Path
import subprocess
import tempfile
import unittest

from check_docs import ROOT, check_links, heading_slug, lint, markdown_files, read_document


class DocumentationChecks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        subprocess.run(["git", "init", "--quiet", str(self.root)], check=True)
        (self.root / ".pymarkdown.json").write_bytes((ROOT / ".pymarkdown.json").read_bytes())

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return path

    def test_git_file_selection(self):
        tracked = self.write("tracked.md", "# Tracked\n")
        subprocess.run(["git", "add", "tracked.md"], cwd=self.root, check=True)
        untracked = self.write("new.MD", "# New\n")
        self.write(".gitignore", "ignored/\n")
        self.write("ignored/generated.md", "# Ignore\n")
        self.write("source.txt", "text\n")
        self.assertEqual(markdown_files(self.root), [untracked, tracked])
        tracked.unlink()
        self.assertIn("missing tracked Markdown file", check_links([tracked], self.root)[0])

    def test_valid_references_images_and_nested_paths(self):
        self.write("docs/target space.md", "# Target `API`, 世界!\n")
        self.write("docs/image.png", "image bytes")
        source = self.write("docs/source.md", """# Links

[inline](target%20space.md#target-api-世界)
[full][target] [collapsed][] [shortcut]
![image](image.png)
[root](/docs/target%20space.md#target-api-%E4%B8%96%E7%95%8C)

[target]: <target space.md#target-api-世界> "A [title](missing.md)"
[collapsed]: target%20space.md
[shortcut]: target%20space.md
""")
        self.assertEqual(check_links([source], self.root), [])
        self.assertEqual(len(read_document(source).links), 6)

    def test_missing_local_files_and_fragments(self):
        self.write("target.md", "# Exists\n")
        source = self.write("source.md", """# Source

[missing](gone.md)
[anchor](target.md#missing)
![image](gone.png)
[local](#missing)
""")
        errors = check_links([source], self.root)
        self.assertEqual(len(errors), 4)
        self.assertIn("source.md:3: missing local target", errors[0])
        self.assertIn("missing Markdown anchor", errors[1])

    def test_code_examples_and_reference_titles_are_not_links(self):
        source = self.write("source.md", """# Examples

`[example](missing-inline.md)`

```markdown
[example](missing-fenced.md)
[ref]: missing-definition.md
```

    [example](missing-indented.md)

[valid](#examples "[example](missing-title.md)")
[unused]: missing-unused.md
""")
        self.assertEqual(check_links([source], self.root), [])
        self.assertEqual(len(read_document(source).links), 1)

    def test_duplicate_formatted_and_setext_headings(self):
        source = self.write("source.md", """# *API* `value`, 世界!

API value, 世界!
----------------

## API value, 世界!-1

## API value, 世界!

[first](#api-value-世界)
[second](#api-value-世界-1)
[explicit suffix](#api-value-世界-1-1)
[third](#api-value-世界-2)
""")
        self.assertEqual(check_links([source], self.root), [])

    def test_unicode_multiline_and_image_headings(self):
        self.assertEqual(heading_slug("Привет non-latin 你好"), "привет-non-latin-你好")
        self.assertEqual(heading_slug("😄 I ♥ e\u0301"), "-i--e\u0301")
        self.write("image.png", "image bytes")
        source = self.write("source.md", """First
second
------

## ![_Image_](image.png) API

> [multiline](#firstsecond)

[image heading](#image-api)
""")
        self.assertEqual(check_links([source], self.root), [])

    def test_html_anchors_links_and_external_schemes(self):
        source = self.write("source.md", """# Links

<a id="custom"></a>

[custom](#custom)
<a href="#links">HTML link</a>
[web](https://invalid.example/missing#missing)
[mail](mailto:nobody@example.com)
[other](custom:thing)
[relative web](//invalid.example/missing)
""")
        self.assertEqual(check_links([source], self.root), [])

    def test_lint_accepts_technical_prose_and_rejects_bad_spacing(self):
        valid = self.write("valid.md", "# Technical prose\n\n" + ("long " * 80).rstrip() +
                           "\n\nhttps://example.com\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n")
        self.assertEqual(lint([valid], self.root), 0)
        invalid = self.write("invalid.md", "# Heading\n\nTrailing spaces   \n")
        self.assertNotEqual(lint([invalid], self.root), 0)


if __name__ == "__main__":
    unittest.main()
