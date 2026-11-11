#!/bin/sh
# STEP 0 of the nsh-share workflow: the references every area reads. Idempotent; exits non-zero if any clone is missing.
set -e
cd /Users/hmziq/fo/nsh-share
mkdir -p .refs
grep -q '^\.refs/$' .gitignore || printf '.refs/\n' >> .gitignore
clone() { [ -d ".refs/$1/.git" ] || git clone --depth 1 --single-branch "https://github.com/$2.git" ".refs/$1"; }
clone plus_plugins fluttercommunity/plus_plugins
clone receive_sharing_intent KasemJaffer/receive_sharing_intent
clone react-native-share react-native-share/react-native-share
clone nsh longbridge/nsh
clone biometrics SuperNSH/biometrics
for r in plus_plugins receive_sharing_intent react-native-share nsh biometrics; do
  [ -f ".refs/$r/.git/HEAD" ] || { echo "missing .refs/$r" >&2; exit 1; }
done
echo "step0 ok: all five references present"
