// Versions of the build plugins. Module build files apply them without versions.
plugins {
    kotlin("jvm") version "2.4.0" apply false
    kotlin("plugin.serialization") version "2.4.0" apply false
    id("com.android.library") version "9.4.1" apply false
}
