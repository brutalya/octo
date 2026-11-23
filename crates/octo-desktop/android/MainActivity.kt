package com.octo.app

import android.graphics.Color
import android.os.Bundle
import android.view.View
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

// Replaces the generated MainActivity (gen/android is not committed; android.yml copies this
// file after `tauri android init`). Android 15+ always draws apps edge to edge and Tauri does
// not handle insets, so the UI ended up under the status bar and the keyboard. The content view
// (which holds the WebView) is padded by the system bars, display cutout and keyboard instead.
class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val content = findViewById<View>(android.R.id.content)
    // Same as the web UI background, so the padded strips match the app.
    content.setBackgroundColor(Color.rgb(19, 19, 21))
    ViewCompat.setOnApplyWindowInsetsListener(content) { v, insets ->
      val b = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or
          WindowInsetsCompat.Type.displayCutout() or
          WindowInsetsCompat.Type.ime()
      )
      v.setPadding(b.left, b.top, b.right, b.bottom)
      WindowInsetsCompat.CONSUMED
    }
    ViewCompat.requestApplyInsets(content)
  }
}
