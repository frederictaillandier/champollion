package com.maujart.champollion

import android.content.Context
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.flow.first
import kotlinx.serialization.SerializationException

/**
 * Sends the reviews and flags made here, then, once the session is done,
 * downloads the next one.
 */
object Sync {
    /** Cards downloaded at once: a session's worth. */
    private const val SESSION_SIZE = 50

    suspend fun run(context: Context) {
        val session = context.session.data.first()
        if (session.pending.isNotEmpty()) {
            Backend.sendReviews(session.pending)
            context.session.updateData { it.sent(session.pending) }
        }
        if (session.flags.isNotEmpty()) {
            Backend.sendFlags(session.flags)
            context.session.updateData { it.flagsSent(session.flags) }
        }
        if (session.reload) {
            context.session.updateData { it.restart() }
        }
        // Topping the session up after each rating would make it endless
        // while the backend has more cards due than a session holds.
        val due = if (context.session.data.first().queue.isEmpty()) {
            Backend.dueCards(SESSION_SIZE)
        } else {
            emptyList()
        }
        context.session.updateData {
            it.merge(due).copy(lastSync = System.currentTimeMillis(), syncError = null)
        }
    }

    /** Syncs as soon as there is a network, retrying until it works. */
    fun soon(context: Context) {
        val request = OneTimeWorkRequestBuilder<SyncWorker>()
            .setConstraints(networkConstraint)
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
            .build()
        WorkManager.getInstance(context)
            .enqueueUniqueWork("sync", ExistingWorkPolicy.REPLACE, request)
    }

    /**
     * Replaces the session's cards by a new download, once the reviews and
     * flags made here are sent. Asked in the session, so a sync replacing
     * this one still does it.
     */
    suspend fun reload(context: Context) {
        context.session.updateData { it.copy(reload = true) }
        soon(context)
    }

    /** Syncs every half hour, so the widgets have cards. */
    fun periodically(context: Context) {
        val request = PeriodicWorkRequestBuilder<SyncWorker>(30, TimeUnit.MINUTES)
            .setConstraints(networkConstraint)
            .build()
        WorkManager.getInstance(context)
            .enqueueUniquePeriodicWork("periodic-sync", ExistingPeriodicWorkPolicy.KEEP, request)
    }

    private val networkConstraint =
        Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()
}

class SyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val result = try {
            Sync.run(applicationContext)
            Result.success()
        } catch (e: IOException) {
            // Usually the WireGuard tunnel is off: try again later.
            fail(e)
            Result.retry()
        } catch (e: SerializationException) {
            fail(e)
            Result.failure()
        }
        updateWidgets(applicationContext)
        return result
    }

    private suspend fun fail(e: Exception) {
        applicationContext.session.updateData {
            it.copy(syncError = e.message ?: e.javaClass.simpleName)
        }
    }
}
