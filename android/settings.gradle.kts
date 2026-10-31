// Thrall for Android: a native client of the engine on the person's computer, over the encrypted
// phone link (docs/protocol.md §13a). Every screen is built here; all of Thrall's work stays on the
// computer.

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "thrall-android"
include(":app")
