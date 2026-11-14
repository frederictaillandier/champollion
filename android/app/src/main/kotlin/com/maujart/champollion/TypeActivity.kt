package com.maujart.champollion

import android.app.Activity
import android.graphics.Color
import android.graphics.Typeface
import android.os.Bundle
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.util.TypedValue
import android.view.Gravity
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.view.WindowManager
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Types the word of the session's current card (level 3) from its
 * translation. The exact word succeeds as soon as it is typed; Fail shows it.
 * Either way the card is rated here and the screen closes, back to the widget.
 */
class TypeActivity : Activity() {
    private val scope: CoroutineScope = MainScope()
    private lateinit var card: Card
    private var hint = -1
    private var answered = false

    private lateinit var pattern: TextView
    private lateinit var input: EditText
    private lateinit var fail: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        scope.launch {
            val session = applicationContext.session.data.first()
            val current = session.current()
            if (current == null || !Typing.isTyped(current) || session.flipped) {
                finish()
                return@launch
            }
            card = current
            hint = Typing.hintIndex(card.lemma, session.typingSeed)
            show()
        }
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun show() {
        val grammar = listOf(card.pos, card.gender).filter { it.isNotEmpty() }.joinToString(", ")
        val layout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            setBackgroundColor(BACKGROUND)
            val pad = dp(24)
            setPadding(pad, dp(64), pad, pad)
        }
        layout.addView(text(grammar, 14f, MUTED))
        layout.addView(text(card.translation, 24f, TEXT, bold = true).apply {
            setPadding(0, dp(8), 0, dp(32))
        })
        pattern = text("", 30f, TEXT, bold = true).apply {
            typeface = Typeface.create(Typeface.MONOSPACE, Typeface.BOLD)
        }
        layout.addView(pattern)
        input = EditText(this).apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            isSingleLine = true
            gravity = Gravity.CENTER
            setTextColor(TEXT)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 20f)
            hint = "Type the word"
            setHintTextColor(MUTED)
            addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
                override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
                override fun afterTextChanged(s: Editable?) = typed(s.toString())
            })
        }
        layout.addView(input, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply {
            topMargin = dp(24)
        })
        fail = Button(this).apply {
            text = "Fail"
            setTextColor(Color.WHITE)
            setBackgroundColor(FAILED)
            setOnClickListener { if (answered) finish() else failed() }
        }
        layout.addView(fail, LinearLayout.LayoutParams(MATCH_PARENT, dp(52)).apply {
            topMargin = dp(24)
        })
        setContentView(layout)
        typed("")

        window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_VISIBLE)
        input.requestFocus()
        input.post {
            getSystemService(InputMethodManager::class.java)
                .showSoftInput(input, InputMethodManager.SHOW_IMPLICIT)
        }
    }

    private fun typed(text: String) {
        if (answered) return
        pattern.text = Typing.pattern(card.lemma, hint, text)
        if (Typing.isExact(text, card.lemma)) succeeded()
    }

    private fun succeeded() {
        answered = true
        input.isEnabled = false
        pattern.text = Typing.pattern(card.lemma, hint, card.lemma)
        pattern.setTextColor(SUCCEEDED)
        fail.isEnabled = false
        scope.launch {
            rate(Rating.Succeeded)
            val minimum = launch { delay(SUCCESS_SHOWN_MS) }
            speak()
            minimum.join()
            finish()
        }
    }

    private fun failed() {
        answered = true
        input.isEnabled = false
        pattern.text = Typing.pattern(card.lemma, hint, card.lemma)
        pattern.setTextColor(FAILED)
        fail.text = "Next"
        fail.setBackgroundColor(NEXT)
        scope.launch {
            rate(Rating.Failed)
            speak()
        }
    }

    /** Rates the card in the session, whether or not this screen stays open. */
    private suspend fun rate(rating: Rating) = withContext(NonCancellable) {
        val session = applicationContext.session.updateData { session ->
            // Only if the card is still the one shown, e.g. not rated meanwhile.
            if (session.current()?.id == card.id) session.rate(rating, System.currentTimeMillis()) else session
        }
        updateWidgets(applicationContext)
        Sync.whenDone(applicationContext, session)
    }

    private suspend fun speak() {
        val muted = applicationContext.session.data.first().muted
        if (!muted) withContext(Dispatchers.IO) { Pronunciations.play(applicationContext, card) }
    }

    private fun text(value: String, size: Float, color: Int, bold: Boolean = false) =
        TextView(this).apply {
            text = value
            gravity = Gravity.CENTER
            setTextColor(color)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, size)
            if (bold) setTypeface(typeface, Typeface.BOLD)
        }

    private fun dp(value: Int): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value.toFloat(), resources.displayMetrics)
            .toInt()

    private companion object {
        /** How long a success shows before the screen closes. */
        const val SUCCESS_SHOWN_MS = 1_000L

        val BACKGROUND = Color.parseColor("#1E1E1E")
        val TEXT = Color.parseColor("#F5F5F5")
        val MUTED = Color.parseColor("#9E9E9E")
        val SUCCEEDED = Color.parseColor("#66BB6A")
        val FAILED = Color.parseColor("#C62828")
        val NEXT = Color.parseColor("#616161")
    }
}
