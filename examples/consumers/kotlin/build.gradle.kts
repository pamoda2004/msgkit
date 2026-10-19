// A separate Gradle project that consumes msgkit from a Maven repository, like any outside project would.
// Until msgkit is published to Maven Central, publish it to your local Maven repository first:
//   cd packages/kotlin && gradlew :msgkit:publishToMavenLocal
plugins {
    kotlin("jvm") version "2.4.21"
    application
}

repositories {
    mavenLocal {
        content { includeGroup("io.github.pamoda2004") }
    }
    mavenCentral()
}

dependencies {
    implementation("io.github.pamoda2004:msgkit:0.2.0")
}

application {
    mainClass.set("AppKt")
    // Recent JDKs warn when a library (JNA) loads a native library; this acknowledges it.
    applicationDefaultJvmArgs = listOf("--enable-native-access=ALL-UNNAMED")
}
