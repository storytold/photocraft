// The Kotlin shell around libphotocraft_mobile.so. Build the library first (see README.md); it
// lands in src/main/jniLibs/arm64-v8a.
plugins { id("com.android.application") }

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
    // The Rust library is already stripped and optimised by cargo.
    packaging { jniLibs { useLegacyPackaging = true; keepDebugSymbols += "**/*.so" } }
}
