plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

android {
    namespace = "com.maujart.champollion"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.maujart.champollion"
        minSdk = 26
        targetSdk = 37
        versionCode = 2
        versionName = "0.2.0"
        // champollion-backend, reachable through the WireGuard tunnel only.
        buildConfigField("String", "BACKEND_URL", "\"http://10.0.0.1:8090\"")
    }

    buildFeatures {
        buildConfig = true
    }
}

dependencies {
    implementation(libs.glance.appwidget)
    implementation(libs.glance.material3)
    implementation(libs.datastore)
    implementation(libs.work.runtime)
    implementation(libs.serialization.json)
    testImplementation(libs.junit)
}
