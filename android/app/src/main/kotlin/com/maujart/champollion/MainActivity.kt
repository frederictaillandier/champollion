package com.maujart.champollion

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.os.Bundle
import kotlinx.coroutines.runBlocking

/**
 * Syncs the cards, and asks the launcher to add the widget if it has none.
 * From the app icon's "Reload cards" shortcut, downloads a new session.
 */
class MainActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Sync.periodically(this)
        if (intent.action == ACTION_RELOAD) {
            // A small DataStore write, done before the activity finishes.
            runBlocking { Sync.reload(applicationContext) }
        } else {
            Sync.soon(this)
        }
    }

    // Only a foreground activity may ask to add a widget, so not in onCreate.
    override fun onResume() {
        super.onResume()
        val manager = AppWidgetManager.getInstance(this)
        val widget = ComponentName(this, ReviewWidgetReceiver::class.java)
        if (manager.getAppWidgetIds(widget).isEmpty() && manager.isRequestPinAppWidgetSupported) {
            manager.requestPinAppWidget(widget, null, null)
        }
        finish()
    }

    companion object {
        const val ACTION_RELOAD = "com.maujart.champollion.RELOAD"
    }
}
