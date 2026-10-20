plugins {
    id("com.android.library")
    `maven-publish`
    id("org.jlleitschuh.gradle.ktlint")
    id("dev.detekt")
    id("org.cyclonedx.bom")
}

val workspaceVersion: String =
    Regex("""(?m)^version = "(.+)"$""")
        .find(rootDir.resolve("../../Cargo.toml").readText())!!
        .groupValues[1]

group = "io.github.pamoda2004"
version = workspaceVersion

android {
    namespace = "io.github.pamoda2004.msgkit.android"
    compileSdk = 36

    defaultConfig {
        // java.time (used by the Instant helpers) needs API 26 without desugaring.
        minSdk = 26
        consumerProguardFiles("consumer-rules.pro")
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }

    publishing {
        singleVariant("release") { withSourcesJar() }
    }
}

// This module only packages `libmsgkit_ffi.so` for each ABI (built into
// src/main/jniLibs by scripts/build_android_libs.py) next to the pure-JVM
// :msgkit module. On Android, JNA must come as an AAR (it carries its own
// libjnidispatch.so), so the JVM flavour of JNA is excluded.
dependencies {
    api(project(":msgkit")) {
        exclude(group = "net.java.dev.jna", module = "jna")
    }
    api("net.java.dev.jna:jna:5.19.1@aar")

    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}

afterEvaluate {
    publishing {
        publications {
            create<MavenPublication>("release") {
                artifactId = "msgkit-android"
                from(components["release"])
                pom {
                    name.set("msgkit for Android")
                    description.set("msgkit with native libraries for Android (arm64-v8a, armeabi-v7a, x86_64, x86).")
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
}

detekt {
    buildUponDefaultConfig = true
    config.setFrom(rootDir.resolve("detekt.yml"))
}

// The software bill of materials of the AAR: what a consumer's build resolves at runtime for the release variant.
tasks.cyclonedxDirectBom {
    includeConfigs.set(listOf("releaseRuntimeClasspath"))
    projectType.set(org.cyclonedx.model.Component.Type.LIBRARY)
    schemaVersion.set(org.cyclonedx.Version.VERSION_15)
    jsonOutput.set(layout.buildDirectory.file("reports/cyclonedx/msgkit-android.cdx.json"))
    xmlOutput.unsetConvention()
}
