import java.io.File
import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// The release key lives outside the repository; android/local.properties (never committed) says
// where and holds its password. Without them the release build is left unsigned.
val machine = Properties().apply {
    val props = rootProject.file("local.properties")
    if (props.exists()) props.reader().use { load(it) }
}
val releaseKeyFile: File? = machine.getProperty("thrall.signing.file")?.let(::File)?.takeIf(File::exists)
val releaseKeyPassword: String? = machine.getProperty("thrall.signing.password")

android {
    namespace = "app.thrall"
    compileSdk = 37

    defaultConfig {
        applicationId = "app.thrall"
        minSdk = 30 // Android 11: a tester's phone; lint (NewApi) guards anything newer
        targetSdk = 37
        // Raise versionCode for every APK that goes to a phone: the app only offers an update that's
        // newer than itself, and Android refuses to install an older one over it.
        versionCode = 7
        versionName = "0.5.0"
    }

    signingConfigs {
        if (releaseKeyFile != null && releaseKeyPassword != null) {
            create("release") {
                storeFile = releaseKeyFile
                storeType = "PKCS12"
                storePassword = releaseKeyPassword
                keyAlias = "thrall"
                keyPassword = releaseKeyPassword
            }
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.findByName("release")
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    packaging {
        resources.excludes += setOf("META-INF/versions/9/OSGI-INF/MANIFEST.MF", "META-INF/DEPENDENCIES")
    }

    testOptions {
        unitTests.all {
            // The protocol drift test reads the engine's schema from the repository. It's an input,
            // so a schema change runs the tests again (otherwise Gradle thinks they're up to date).
            val schema = rootProject.file("../docs/protocol.schema.json")
            it.inputs.file(schema).withPathSensitivity(PathSensitivity.NONE)
            it.systemProperty("thrall.schema", schema.absolutePath)
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons)
    implementation(libs.camera.camera2)
    implementation(libs.camera.lifecycle)
    implementation(libs.camera.view)
    implementation(libs.okhttp)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.zxing.core)
    implementation(libs.bouncycastle)
    debugImplementation(libs.compose.ui.tooling)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
}
