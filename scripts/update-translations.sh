#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."
xgettext --language=Rust --from-code=UTF-8 --keyword=tr:1 --keyword=button:3 --keyword=ngettext:2,3 --package-name=tg-player --sort-output -o assets/i18n/tg-player.pot $(find src -name '*.rs' -not -path '*/tests/*' -not -name '*_tests.rs')
for catalog in assets/i18n/*.po; do
    msgmerge --update --backup=none "$catalog" assets/i18n/tg-player.pot
    msgfmt --check "$catalog" -o /dev/null
done
