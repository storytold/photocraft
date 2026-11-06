// The Compose shell around libphotocraft_mobile.so. Build the library first, from the repo root:
//   cargo ndk -t arm64-v8a -P 26 -o apps/photocraft-mobile/android/app/src/main/jniLibs build --release -p photocraft-mobile
plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "dev.craft.photocraft"
    compileSdk = 37
    defaultConfig {
        applicationId = "dev.craft.photocraft"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1"
        ndk { abiFilters += "arm64-v8a" }
    }
    buildFeatures { compose = true }
    // The Rust library is already stripped and optimised by cargo.
    packaging { jniLibs { useLegacyPackaging = true; keepDebugSymbols += "**/*.so" } }
}

dependencies {
    implementation(project(":craftkit"))
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.activity:activity-compose:1.13.0")
}
