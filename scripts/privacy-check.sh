#!/usr/bin/env bash
# Fails if the tree (or a range of commits) holds anything private: personal email addresses,
# home folders, private network addresses, or commits by a personal address.
# Usage: scripts/privacy-check.sh [<base-commit>]   (with a base, the commits since it are checked too)
# A maintainer can keep extra words (names, user names) in .git/info/private-words, one per line,
# and extra file patterns in .git/info/private-files (one extended regex per line); both stay on
# that computer and are never committed.
set -u
cd "$(dirname "$0")/.."
fail=0
say() { echo "privacy: $*" >&2; fail=1; }

# Emails: only GitHub's no-reply addresses, the project's contact address and examples.
emails=$(git grep -I -n -o -E '[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}' -- \
  ':!*.lock' ':!**/package-lock.json' |
  grep -v -i -E '@(users\.noreply\.github\.com|github\.com|example\.(com|org|net))$|thrallai@protonmail\.com$|@[0-9]+x\.png$' || true)
[ -n "$emails" ] && say "email addresses found:"$'\n'"$emails"

# Home folders other than the placeholders used in docs and tests.
homes=$(git grep -I -n -P '(?i)[A-Z]:\\{1,2}Users\\{1,2}(?!you\b|me\b|Public\b|name\b|<|\{|%|\$)[A-Za-z0-9._-]+|/home/(?!you\b|me\b|user\b|runner\b|\$)[a-z0-9._-]+/|/Users/(?!you\b|me\b|runner\b|Shared\b|\$)[A-Za-z0-9._-]+/' || true)
[ -n "$homes" ] && say "personal home folders found:"$'\n'"$homes"

# Tailscale addresses and machine names (example.ts.net is the placeholder).
tail=$(git grep -I -n -P '\b100\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.[0-9]{1,3}\.[0-9]{1,3}\b|\b[a-z0-9-]+\.(?!example\.|your-tailnet\.)[a-z0-9-]+\.ts\.net\b' || true)
[ -n "$tail" ] && say "private network addresses found:"$'\n'"$tail"

# This computer's own lists, if it has them.
if [ -f .git/info/private-words ]; then
  while IFS= read -r word; do
    word=${word%$'\r'}
    [ -z "$word" ] && continue
    hits=$(git grep -I -n -i -F -e "$word" || true)
    [ -n "$hits" ] && say "a private word was found:"$'\n'"$hits"
    if [ $# -ge 1 ]; then
      inlog=$(git log --format='%h %an %ae %cn %ce %B' "$1..HEAD" | grep -i -F -e "$word" || true)
      [ -n "$inlog" ] && say "a private word is in a commit:"$'\n'"$inlog"
    fi
  done < .git/info/private-words
fi
if [ -f .git/info/private-files ]; then
  while IFS= read -r pattern; do
    pattern=${pattern%$'\r'}
    [ -z "$pattern" ] && continue
    hits=$(git ls-files | grep -E -i -e "$pattern" || true)
    [ -n "$hits" ] && say "files that must stay local are tracked:"$'\n'"$hits"
  done < .git/info/private-files
fi

# Commits since a base: author and committer addresses, and no co-author lines. GitHub's own
# merge commits (made when a pull request is tested or merged) are left out of the address check,
# and the project's public contact address is fine.
if [ $# -ge 1 ]; then
  bad=$(git log --no-merges --format='%h %ae %ce' "$1..HEAD" |
    awk '{ for (i = 2; i <= 3; i++) { a = tolower($i); if (a !~ /@users\.noreply\.github\.com$/ && a != "noreply@github.com" && a != "thrallai@protonmail.com") { print; next } } }')
  [ -n "$bad" ] && say "commits by an address that isn't a GitHub no-reply one:"$'\n'"$bad"
  coauth=$(git log --no-merges --format='%h %B' "$1..HEAD" | grep -i 'co-authored-by' || true)
  [ -n "$coauth" ] && say "commit messages carry co-author lines:"$'\n'"$coauth"
fi

[ "$fail" -eq 0 ] && echo "privacy: nothing private found."
exit "$fail"
