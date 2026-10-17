import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    kotlin("jvm")
    kotlin("plugin.serialization")
    `maven-publish`
    id("org.jlleitschuh.gradle.ktlint")
    id("dev.detekt")
}

// The Kotlin package is released in lock-step with the Rust workspace.
val workspaceVersion: String =
    Regex("""(?m)^version = "(.+)"$""")
        .find(rootDir.resolve("../../Cargo.toml").readText())!!
        .groupValues[1]

group = "io.github.pamoda2004"
version = workspaceVersion

kotlin {
    explicitApi()
    compilerOptions { jvmTarget.set(JvmTarget.JVM_11) }
}

java {
    sourceCompatibility = JavaVersion.VERSION_11
    targetCompatibility = JavaVersion.VERSION_11
    withSourcesJar()
}

tasks.withType<JavaCompile>().configureEach { options.release.set(11) }

// The bundled natives belong in the binary JAR only, not in the sources JAR.
tasks.named<Jar>("sourcesJar") { exclude("**/*.dll", "**/*.so", "**/*.dylib") }

dependencies {
    // Exposed in the public API (the native interface is internal, but JNA types
    // surface in stack traces and consumers may need to configure it).
    api("net.java.dev.jna:jna:5.19.1")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.11.0")

    testImplementation(kotlin("test"))
}

// Native libraries for desktop/server JVMs are bundled in the JAR under JNA's
// resource layout (`<os>-<arch>/<library>`), e.g. `win32-x86-64/msgkit_ffi.dll`.
// The release workflow fills `natives/`; a plain checkout has none and falls
// back to jna.library.path / MSGKIT_LIB.
sourceSets.main { resources.srcDir("natives") }

tasks.test {
    useJUnitPlatform()
    val repo = rootDir.resolve("../..")
    systemProperty("msgkit.fixtures", repo.resolve("crates/msgkit-core/tests/fixtures").absolutePath)
    systemProperty("msgkit.workspaceVersion", workspaceVersion)
    systemProperty("msgkit.conformance", repo.resolve("conformance/cases").absolutePath)
    // Use the freshly built native library from the Cargo target directory.
    val libDirs = listOf("release", "debug").map { repo.resolve("target/$it") }.filter { it.isDirectory }
    systemProperty("jna.library.path", libDirs.joinToString(File.pathSeparator) { it.absolutePath })
    testLogging {
        events("passed", "failed", "skipped")
        showStandardStreams = false
    }
}

publishing {
    publications {
        create<MavenPublication>("maven") {
            artifactId = "msgkit"
            from(components["java"])
            pom {
                name.set("msgkit")
                description.set("Import SMS, WhatsApp and Messenger messages and calls into one normalized model.")
                url.set("https://github.com/pamoda2004/msgkit")
                licenses {
                    license {
                        name.set("MIT")
                        url.set("https://opensource.org/licenses/MIT")
                    }
                }
                developers {
                    developer {
                        id.set("pamoda2004")
                        name.set("Pamoda Jayathilaka")
                    }
                }
                scm {
                    url.set("https://github.com/pamoda2004/msgkit")
                    connection.set("scm:git:https://github.com/pamoda2004/msgkit.git")
                }
            }
        }
    }
}

detekt {
    buildUponDefaultConfig = true
    config.setFrom(rootDir.resolve("detekt.yml"))
}
