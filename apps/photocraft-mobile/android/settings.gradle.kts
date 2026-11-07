pluginManagement { repositories { google(); mavenCentral(); gradlePluginPortal() } }
dependencyResolutionManagement { repositories { google(); mavenCentral() } }
rootProject.name = "PhotoCraft"
include(":app")
// The shared Compose shell: craft-libs (crates/mobile), checked out beside the Craft repos at the
// commit the workspace Cargo.toml pins for craft-mobile, or wherever CRAFT_MOBILE_DIR points.
val craftMobile = System.getenv("CRAFT_MOBILE_DIR") ?: "../../../../craft-libs/crates/mobile"
include(":craftkit")
project(":craftkit").projectDir = file("$craftMobile/android/craftkit")
