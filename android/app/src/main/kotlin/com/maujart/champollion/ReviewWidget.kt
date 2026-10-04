package com.maujart.champollion

import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.datastore.core.DataStore
import androidx.datastore.dataStoreFile
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.GlanceTheme
import androidx.glance.action.ActionParameters
import androidx.glance.action.actionParametersOf
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetManager
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.SizeMode
import androidx.glance.appwidget.action.ActionCallback
import androidx.glance.appwidget.action.actionRunCallback
import androidx.glance.appwidget.appWidgetBackground
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.provideContent
import androidx.glance.background
import androidx.glance.currentState
import androidx.glance.layout.Alignment
import androidx.glance.layout.Box
import androidx.glance.layout.Column
import androidx.glance.layout.ColumnScope
import androidx.glance.layout.Row
import androidx.glance.layout.RowScope
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.height
import androidx.glance.layout.padding
import androidx.glance.state.GlanceStateDefinition
import androidx.glance.text.FontStyle
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextAlign
import androidx.glance.text.TextStyle
import androidx.glance.unit.ColorProvider
import java.io.File

/** Every widget shows the one [Session]. */
object SessionStateDefinition : GlanceStateDefinition<Session> {
    override suspend fun getDataStore(context: Context, fileKey: String): DataStore<Session> =
        context.session

    override fun getLocation(context: Context, fileKey: String): File =
        context.dataStoreFile(SESSION_FILE)
}

/**
 * A flashcard: the word and the sentence it was read in; tapped, its
 * translation and meaning there, with Anki's rating buttons.
 */
class ReviewWidget : GlanceAppWidget() {
    override val stateDefinition = SessionStateDefinition
    override val sizeMode = SizeMode.Exact

    override suspend fun provideGlance(context: Context, id: GlanceId) {
        provideContent {
            GlanceTheme {
                Content(currentState())
            }
        }
    }
}

abstract class SessionWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = ReviewWidget()

    override fun onEnabled(context: Context) {
        super.onEnabled(context)
        Sync.periodically(context)
        Sync.soon(context)
    }
}

class ReviewWidgetReceiver : SessionWidgetReceiver()

/** The same widget for the Galaxy Z Flip's cover screen (Flex Window). */
class CoverWidgetReceiver : SessionWidgetReceiver()

/**
 * Redraws every widget placed, on both screens, after the session changed.
 * Glance's updateAll() misses widgets placed before Glance knew their
 * receiver (e.g. the cover screen's, kept across app updates).
 */
suspend fun updateWidgets(context: Context) {
    val manager = AppWidgetManager.getInstance(context)
    val glance = GlanceAppWidgetManager(context)
    val widget = ReviewWidget()
    for (receiver in listOf(ReviewWidgetReceiver::class.java, CoverWidgetReceiver::class.java)) {
        for (id in manager.getAppWidgetIds(ComponentName(context, receiver))) {
            widget.update(context, glance.getGlanceIdBy(id))
        }
    }
}

@Composable
private fun Content(session: Session) {
    val now = System.currentTimeMillis()
    val card = session.current(now)
    Column(
        modifier = GlanceModifier
            .fillMaxSize()
            .appWidgetBackground()
            .background(GlanceTheme.colors.widgetBackground)
            .cornerRadius(16.dp)
            .padding(12.dp),
    ) {
        Status(session)
        when {
            card == null -> Empty(session, now)
            session.flipped -> Back(card)
            else -> Front(card)
        }
    }
}

@Composable
private fun Status(session: Session) {
    val parts = buildList {
        add("${session.queue.size} left")
        if (session.pending.isNotEmpty()) add("${session.pending.size} to send")
        if (session.syncError != null) add("offline")
    }
    Text(
        text = parts.joinToString(" · "),
        style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 11.sp),
    )
}

@Composable
private fun ColumnScope.Front(card: Card) {
    val sighting = card.sightings.firstOrNull()
    Column(
        modifier = GlanceModifier
            .defaultWeight()
            .fillMaxWidth()
            .clickable(actionRunCallback<FlipAction>()),
        verticalAlignment = Alignment.CenterVertically,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = card.lemma,
            style = TextStyle(
                color = GlanceTheme.colors.onSurface,
                fontSize = 26.sp,
                fontWeight = FontWeight.Bold,
                textAlign = TextAlign.Center,
            ),
        )
        if (sighting != null) {
            Spacer(GlanceModifier.height(8.dp))
            Text(
                text = sighting.sentence,
                maxLines = 4,
                style = TextStyle(
                    color = GlanceTheme.colors.onSurface,
                    fontSize = 14.sp,
                    textAlign = TextAlign.Center,
                ),
            )
        }
        Spacer(GlanceModifier.height(8.dp))
        Text(
            text = "Tap to see the answer",
            style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 11.sp),
        )
    }
}

@Composable
private fun ColumnScope.Back(card: Card) {
    val sighting = card.sightings.firstOrNull()
    Column(
        modifier = GlanceModifier
            .defaultWeight()
            .fillMaxWidth()
            .clickable(actionRunCallback<FlipAction>()),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = listOf(card.lemma, grammar(card)).filter { it.isNotEmpty() }.joinToString(" · "),
            style = TextStyle(
                color = GlanceTheme.colors.onSurface,
                fontSize = 15.sp,
                fontWeight = FontWeight.Bold,
            ),
        )
        Text(
            text = card.translation,
            maxLines = 2,
            style = TextStyle(
                color = GlanceTheme.colors.primary,
                fontSize = 18.sp,
                fontWeight = FontWeight.Medium,
            ),
        )
        if (sighting != null) {
            Spacer(GlanceModifier.height(4.dp))
            Text(
                text = sighting.definition,
                maxLines = 3,
                style = TextStyle(color = GlanceTheme.colors.onSurface, fontSize = 13.sp),
            )
            Spacer(GlanceModifier.height(4.dp))
            Text(
                text = sighting.sentenceTranslation,
                maxLines = 3,
                style = TextStyle(
                    color = GlanceTheme.colors.onSurfaceVariant,
                    fontSize = 12.sp,
                    fontStyle = FontStyle.Italic,
                ),
            )
        }
    }
    Row(modifier = GlanceModifier.fillMaxWidth()) {
        Rating.entries.forEach { RateButton(it) }
    }
}

/** e.g. `noun, m` */
private fun grammar(card: Card): String =
    listOf(card.pos, card.gender).filter { it.isNotEmpty() }.joinToString(", ")

private val buttonColors = mapOf(
    Rating.Again to Color(0xFFC62828),
    Rating.Hard to Color(0xFFEF6C00),
    Rating.Good to Color(0xFF2E7D32),
    Rating.Easy to Color(0xFF1565C0),
)

@Composable
private fun RowScope.RateButton(rating: Rating) {
    Box(modifier = GlanceModifier.defaultWeight().height(40.dp).padding(horizontal = 2.dp)) {
        Box(
            modifier = GlanceModifier
                .fillMaxSize()
                .background(buttonColors.getValue(rating))
                .cornerRadius(8.dp)
                .clickable(
                    actionRunCallback<RateAction>(actionParametersOf(RatingKey to rating.name)),
                ),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = rating.label,
                maxLines = 1,
                style = TextStyle(
                    color = ColorProvider(Color.White),
                    fontSize = 13.sp,
                    fontWeight = FontWeight.Bold,
                ),
            )
        }
    }
}

@Composable
private fun ColumnScope.Empty(session: Session, now: Long) {
    val next = session.nextAt(now)
    val title = when {
        next != null -> "Next card in ${(next - now) / 60_000 + 1} min"
        session.lastSync == 0L && session.syncError != null -> "Can't reach the server"
        session.lastSync == 0L -> "Loading cards…"
        else -> "No cards due"
    }
    val hint = session.syncError?.let { "Is the VPN on? ($it)" } ?: "Tap to refresh"
    Column(
        modifier = GlanceModifier
            .defaultWeight()
            .fillMaxWidth()
            .clickable(actionRunCallback<RefreshAction>()),
        verticalAlignment = Alignment.CenterVertically,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = title,
            style = TextStyle(
                color = GlanceTheme.colors.onSurface,
                fontSize = 16.sp,
                fontWeight = FontWeight.Medium,
                textAlign = TextAlign.Center,
            ),
        )
        Spacer(GlanceModifier.height(4.dp))
        Text(
            text = hint,
            maxLines = 3,
            style = TextStyle(
                color = GlanceTheme.colors.onSurfaceVariant,
                fontSize = 12.sp,
                textAlign = TextAlign.Center,
            ),
        )
    }
}

private val RatingKey = ActionParameters.Key<String>("rating")

class FlipAction : ActionCallback {
    override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
        context.session.updateData { it.flip() }
        updateWidgets(context)
    }
}

class RateAction : ActionCallback {
    override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
        val rating = Rating.valueOf(parameters[RatingKey] ?: return)
        context.session.updateData { it.rate(rating, System.currentTimeMillis()) }
        updateWidgets(context)
        Sync.soon(context)
    }
}

class RefreshAction : ActionCallback {
    override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
        Sync.soon(context)
    }
}
