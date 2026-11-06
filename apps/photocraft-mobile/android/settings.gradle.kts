pluginManagement { repositories { google(); mavenCentral(); gradlePluginPortal() } }
dependencyResolutionManagement { repositories { google(); mavenCentral() } }
rootProject.name = "PhotoCraft"
include(":app")
// The shared Compose shell, from the craft-mobile checkout beside the Craft repos.
include(":craftkit")
project(":craftkit").projectDir = file("../../../../../craft-mobile/android/craftkit")
