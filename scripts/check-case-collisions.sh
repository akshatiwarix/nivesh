#!/bin/sh
# Fails if two tracked files differ only in letter case (a path, or a module
# name once the extension is dropped), which breaks on case-insensitive
# filesystems such as macOS's. Same-case siblings (Foo.tsx and Foo.css) are fine.
git ls-files | sed 's/\.[^./]*$//' | sort -u | awk '
  { k = tolower($0); if (k in seen && seen[k] != $0) { print "case-only clash: " seen[k] " vs " $0; bad = 1 } seen[k] = $0 }
  END { exit bad }'
