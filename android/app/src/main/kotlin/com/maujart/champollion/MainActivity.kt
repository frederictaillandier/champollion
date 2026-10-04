package com.maujart.champollion

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.os.Bundle

/** Syncs the cards, and asks the launcher to add the widget if it has none. */
class MainActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Sync.periodically(this)
        Sync.soon(this)
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
}
