import com.vanniktech.maven.publish.SonatypeHost

plugins {
    id("com.android.library") version "8.7.3"
    id("org.jetbrains.kotlin.android") version "2.0.21"
    id("com.vanniktech.maven.publish") version "0.30.0"
}

val waylingoVersion = providers.gradleProperty("waylingoVersion").getOrElse("0.0.0-SNAPSHOT")
val waylingoGroup = providers.gradleProperty("waylingoGroup").getOrElse("io.waylingo")
val repoUrl = "https://github.com/" + providers.environmentVariable("GITHUB_REPOSITORY").getOrElse("waylingo/waylingo")

android {
    namespace = "io.waylingo"
    compileSdk = 35
    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    jvmToolchain(17)
}

dependencies {
    // The generated bindings call the native library through JNA.
    api("net.java.dev.jna:jna:5.14.0@aar")
    api("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.1")
}

mavenPublishing {
    publishToMavenCentral(SonatypeHost.CENTRAL_PORTAL)
    signAllPublications()
    coordinates(waylingoGroup, "waylingo-android", waylingoVersion)
    pom {
        name.set("Waylingo for Android")
        description.set("Plan a trip from one natural-language request: route, stops along it, and a GeoJSON result.")
        url.set(repoUrl)
        licenses {
            license {
                name.set("MIT")
                url.set("https://opensource.org/licenses/MIT")
            }
            license {
                name.set("Apache-2.0")
                url.set("https://www.apache.org/licenses/LICENSE-2.0")
            }
        }
        developers {
            developer {
                name.set("Waylingo contributors")
            }
        }
        scm {
            url.set(repoUrl)
        }
    }
}
