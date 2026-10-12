#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["markdown-it-py==3.0.0", "pymarkdownlnt==0.9.40"]
# ///
"""Lint repository Markdown and check local file/heading links (no network requests)."""

import argparse
from dataclasses import dataclass
from html.parser import HTMLParser
from pathlib import Path
import subprocess
import sys
import unicodedata
from urllib.parse import unquote, urlsplit

from markdown_it import MarkdownIt

ROOT = Path(__file__).resolve().parents[1]
PARSER = MarkdownIt("commonmark").enable(["table", "strikethrough"])


class HtmlAttributes(HTMLParser):
    """Read explicit HTML anchors and links without interpreting HTML as Markdown."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.anchors = set()
        self.links = []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if attrs.get("id"):
            self.anchors.add(attrs["id"])
        if tag == "a" and attrs.get("name"):
            self.anchors.add(attrs["name"])
        for attribute in ("href", "src"):
            if attrs.get(attribute):
                self.links.append(attrs[attribute])


@dataclass
class Document:
    anchors: set[str]
    links: list[tuple[int, str]]


def heading_slug(text):
    # GitHub preserves letters, marks, numbers, hyphens and underscores. Spaces
    # become hyphens; other punctuation (including inline code backticks) vanishes.
    return "".join(
        char if char in "-_" or unicodedata.category(char)[0] in "LMN" else
        "-" if char == " " else ""
        for char in text.lower()
    )


def heading_text(children):
    return "".join(
        heading_text(child.children) if child.type == "image" else child.content
        for child in children or []
        if child.type in ("text", "code_inline", "image")
    )


def read_document(path):
    tokens = PARSER.parse(path.read_text(encoding="utf-8"))
    anchors, links, used = set(), [], set()
    for index, token in enumerate(tokens):
        line = token.map[0] + 1 if token.map else 1
        children = token.children or []
        for child in children:
            if child.type in ("link_open", "image"):
                links.append((line, child.attrGet("href") or child.attrGet("src")))
        html = HtmlAttributes()
        if token.type == "html_block":
            html.feed(token.content)
        for child in children:
            if child.type == "html_inline":
                html.feed(child.content)
        anchors.update(html.anchors)
        links.extend((line, link) for link in html.links)
        if token.type != "heading_open":
            continue
        inline = tokens[index + 1]
        base = heading_slug(heading_text(inline.children))
        slug, suffix = base, 0
        while slug in used:
            suffix += 1
            slug = f"{base}-{suffix}"
        used.add(slug)
        anchors.add(slug)
    return Document(anchors, links)


def markdown_files(root):
    result = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=root, stdout=subprocess.PIPE, check=True,
    )
    return sorted({root / name for name in result.stdout.decode().split("\0")
                   if name.lower().endswith(".md")})


def check_links(paths, root):
    documents = {}
    errors = []

    def document(path):
        if path not in documents:
            documents[path] = read_document(path)
        return documents[path]

    for path in paths:
        if not path.is_file():
            errors.append(f"{path.relative_to(root)}: missing tracked Markdown file")
            continue
        for line, link in document(path).links:
            url = urlsplit(link)
            if url.scheme or url.netloc:
                continue
            local_path = unquote(url.path)
            target = ((root / local_path.lstrip("/")) if local_path.startswith("/")
                      else (path.parent / local_path)) if local_path else path
            target = target.resolve()
            message = None
            if not target.exists():
                message = "missing local target"
            elif url.fragment and target.is_file() and target.suffix.lower() == ".md":
                if unquote(url.fragment) not in document(target).anchors:
                    message = "missing Markdown anchor"
            if message:
                errors.append(f"{path.relative_to(root)}:{line}: {message}: {link}")
    return errors


def lint(paths, root):
    return subprocess.run(
        [sys.executable, "-m", "pymarkdown", "--config", str(root / ".pymarkdown.json"),
         "scan", *map(str, paths)], cwd=root, check=False,
    ).returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test", action="store_true", help="run checker regression tests")
    args = parser.parse_args()
    if args.test:
        import unittest
        suite = unittest.defaultTestLoader.discover(str(ROOT / "tools"), "test_check_docs.py")
        return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1
    paths = markdown_files(ROOT)
    if not paths:
        raise RuntimeError("No repository Markdown files found")
    existing = [path for path in paths if path.is_file()]
    lint_status = lint(existing, ROOT) if existing else 0
    errors = check_links(paths, ROOT)
    for error in errors:
        print(error, file=sys.stderr)
    if lint_status or errors:
        return 1
    print(f"Markdown lint and local links passed ({len(paths)} files; external URLs not checked)")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Documentation check failed: {error}", file=sys.stderr)
        sys.exit(1)
