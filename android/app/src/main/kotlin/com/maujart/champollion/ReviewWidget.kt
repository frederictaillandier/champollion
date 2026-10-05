package com.maujart.champollion

import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Context
import android.os.Build
import android.util.SizeF
import android.widget.RemoteViews
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.datastore.core.DataStore
import androidx.datastore.dataStoreFile
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.GlanceTheme
import androidx.glance.action.Action
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
import androidx.glance.appwidget.compose
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
import androidx.glance.layout.width
import androidx.glance.state.GlanceStateDefinition
import androidx.glance.text.FontStyle
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextAlign
import androidx.glance.text.TextStyle
import androidx.glance.unit.ColorProvider
import java.io.File
import kotlinx.coroutines.flow.first

/** Every widget shows the one [Session]. */
object SessionStateDefinition : GlanceStateDefinition<Session> {
    override suspend fun getDataStore(context: Context, fileKey: String): DataStore<Session> =
        context.session

    override fun getLocation(context: Context, fileKey: String): File =
        context.dataStoreFile(SESSION_FILE)
}

/**
 * A flashcard: the word and the sentence it was read in; tapped, its
 * translation and meaning there, with buttons to rate it.
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
 *
 * The widgets are drawn here and handed to the launcher at once: Glance's
 * update() draws them in a WorkManager job, which Android may hold back for
 * half a minute (e.g. right after unlocking), leaving taps unanswered.
 */
suspend fun updateWidgets(context: Context) {
    val manager = AppWidgetManager.getInstance(context)
    val glance = GlanceAppWidgetManager(context)
    val widget = ReviewWidget()
    val session = context.session.data.first()
    for (receiver in listOf(ReviewWidgetReceiver::class.java, CoverWidgetReceiver::class.java)) {
        for (id in manager.getAppWidgetIds(ComponentName(context, receiver))) {
            val glanceId = glance.getGlanceIdBy(id)
            val options = manager.getAppWidgetOptions(id)
            val sizes = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                options.getParcelableArrayList(AppWidgetManager.OPTION_APPWIDGET_SIZES, SizeF::class.java)
            } else {
                null
            }
            // One layout per size the launcher may show it at, as SizeMode.Exact does.
            val views = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && !sizes.isNullOrEmpty()) {
                RemoteViews(
                    sizes.associateWith {
                        widget.compose(context, glanceId, options, DpSize(it.width.dp, it.height.dp), session)
                    },
                )
            } else {
                widget.compose(context, glanceId, options, state = session)
            }
            manager.updateAppWidget(id, views)
        }
    }
}

@Composable
private fun Content(session: Session) {
    val card = session.current()
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
            card == null -> Empty(session)
            session.flipped -> Back(card, session.muted)
            else -> Front(card)
        }
    }
}

@Composable
private fun Status(session: Session) {
    val parts = buildList {
        add("${session.queue.size} left")
        val unsent = session.pending.size + session.flags.size
        if (unsent > 0) add("$unsent to send")
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
private fun ColumnScope.Back(card: Card, muted: Boolean) {
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
        FlagButton()
        MuteButton(muted)
    }
}

/** e.g. `noun, m` */
private fun grammar(card: Card): String =
    listOf(card.pos, card.gender).filter { it.isNotEmpty() }.joinToString(", ")

private val buttonColors = mapOf(
    Rating.Failed to Color(0xFFC62828),
    Rating.Succeeded to Color(0xFF2E7D32),
)

@Composable
private fun RowScope.RateButton(rating: Rating) {
    Button(
        label = rating.label,
        color = buttonColors.getValue(rating),
        action = actionRunCallback<RateAction>(actionParametersOf(RatingKey to rating.name)),
        modifier = GlanceModifier.defaultWeight(),
    )
}

/** Takes a wrong card out of the reviews. */
@Composable
private fun FlagButton() {
    Button(
        label = "⚑",
        color = Color(0xFF616161),
        action = actionRunCallback<FlagAction>(),
        modifier = GlanceModifier.width(48.dp),
    )
}

/** Stops speaking the words when their card shows, or starts again. */
@Composable
private fun MuteButton(muted: Boolean) {
    Button(
        label = if (muted) "🔇" else "🔊",
        color = Color(0xFF616161),
        action = actionRunCallback<MuteAction>(),
        modifier = GlanceModifier.width(48.dp),
    )
}

@Composable
private fun Button(label: String, color: Color, action: Action, modifier: GlanceModifier) {
    Box(modifier = modifier.height(40.dp).padding(horizontal = 2.dp)) {
        Box(
            modifier = GlanceModifier
                .fillMaxSize()
                .background(color)
                .cornerRadius(8.dp)
                .clickable(action),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = label,
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
private fun ColumnScope.Empty(session: Session) {
    val title = when {
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
        val session = context.session.updateData { it.rate(rating, System.currentTimeMillis()) }
        updateWidgets(context)
        Sync.whenDone(context, session)
        speak(context, session)
    }
}

class FlagAction : ActionCallback {
    override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
        val session = context.session.updateData { it.flag(System.currentTimeMillis()) }
        updateWidgets(context)
        Sync.whenDone(context, session)
        speak(context, session)
    }
}

class MuteAction : ActionCallback {
    override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
        context.session.updateData { it.toggleMute() }
        updateWidgets(context)
    }
}

/**
 * Says the word of the card now shown, unless muted. Only after a tap: the
 * first card of a session, shown by a sync, stays silent.
 */
private suspend fun speak(context: Context, session: Session) {
    if (session.muted) return
    session.current()?.let { Pronunciations.play(context, it) }
}

class RefreshAction : ActionCallback {
    override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
        Sync.soon(context)
    }
}
