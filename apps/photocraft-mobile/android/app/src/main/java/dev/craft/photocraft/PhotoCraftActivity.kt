package dev.craft.photocraft

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dev.craft.kit.CraftHost
import dev.craft.kit.CraftScaffold
import dev.craft.kit.CraftTheme

/** PhotoCraft on Android: craftkit's shell around the Rust app, plus a New button. */
class PhotoCraftActivity : ComponentActivity() {
    private lateinit var host: CraftHost

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        host = CraftHost(this, library = "photocraft_mobile")
        // "Open with" / Share launched us. A recreated activity's intent was handled before, unless
        // the process died meanwhile: the app starts again without it.
        if (savedInstanceState == null || !launchOpened) host.openIntent(intent)
        launchOpened = true
        setContent {
            CraftTheme {
                // PhotoCraft draws its own menus and panels: no native chrome on top.
                CraftScaffold(host, appName = "PhotoCraft")
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        host.openIntent(intent)
    }

    override fun onDestroy() {
        // Leaving for good stops the app; a recreation only lets go of this activity's listeners,
        // and the new activity finds the app still running.
        if (isFinishing) host.close() else host.release()
        super.onDestroy()
    }

    private companion object {
        /** Whether this process has opened a launch intent's documents (reset when it dies). */
        var launchOpened = false
    }
}
