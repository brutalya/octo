package com.octo.app

import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ColorFilter
import android.graphics.Paint
import android.graphics.PixelFormat
import android.graphics.drawable.Drawable
import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

// Replaces the generated MainActivity (gen/android is not committed; android.yml copies this
// file after `tauri android init`). Android 15+ always draws apps edge to edge and Tauri does
// not handle insets, so the UI ended up under the status bar and the keyboard. The content view
// (which holds the WebView) is padded by the system bars, display cutout and keyboard instead.
class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    // As in the generated file: edge to edge on every Android version, padded below.
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    val content = findViewById<View>(android.R.id.content)
    val strips = BarStrips()
    content.background = strips
    ViewCompat.setOnApplyWindowInsetsListener(content) { v, insets ->
      val b = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or
          WindowInsetsCompat.Type.displayCutout() or
          WindowInsetsCompat.Type.ime()
      )
      v.setPadding(b.left, b.top, b.right, b.bottom)
      strips.top = b.top
      strips.invalidateSelf()
      WindowInsetsCompat.CONSUMED
    }
    ViewCompat.requestApplyInsets(content)
  }
}

// Paints the padded strips in the web UI's colors: under the status bar the top bar's color
// (`bg-white`, 27 27 31), elsewhere the page background (`bg-slate-50`, 19 19 21).
private class BarStrips : Drawable() {
  var top = 0
  private val topPaint = Paint().apply { color = Color.rgb(27, 27, 31) }
  private val restPaint = Paint().apply { color = Color.rgb(19, 19, 21) }

  override fun draw(canvas: Canvas) {
    val r = bounds
    val split = (r.top + top).toFloat()
    canvas.drawRect(r.left.toFloat(), r.top.toFloat(), r.right.toFloat(), split, topPaint)
    canvas.drawRect(r.left.toFloat(), split, r.right.toFloat(), r.bottom.toFloat(), restPaint)
  }

  override fun setAlpha(alpha: Int) {}

  override fun setColorFilter(colorFilter: ColorFilter?) {}

  @Deprecated("Deprecated in Java")
  override fun getOpacity(): Int = PixelFormat.OPAQUE
}
