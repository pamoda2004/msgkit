#!/bin/sh
# Runs the Android instrumented tests inside the emulator job and, when they fail, reports the tail of the
# Gradle output as an annotation (job logs are not readable without signing in to GitHub).
cd packages/kotlin || exit 1
if ./gradlew :msgkit-android:connectedDebugAndroidTest --console=plain > ../../android-test.log 2>&1; then
  tail -n 5 ../../android-test.log
  exit 0
fi
tail -n 60 ../../android-test.log
echo "::error title=Android tests failed::$(tail -n 25 ../../android-test.log | tr -s "[:space:]" " " | cut -c1-1800)"
exit 1
