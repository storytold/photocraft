plugins {
    id("com.android.application")
}

android {
    namespace = "ai.storyteller.photocraft"
    compileSdk = 35

    defaultConfig {
        applicationId = "ai.storyteller.photocraft"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.6.0"
        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    sourceSets.getByName("main") {
        manifest.srcFile("../src/main/AndroidManifest.xml")
        java.srcDirs("../src/main/java")
        jniLibs.srcDirs("../src/main/jniLibs")
    }

    buildTypes {
        getByName("release") {
            isMinifyEnabled = false
        }
    }
}
