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
        // "Open with" / Share launched us; a recreated activity's intent was handled before.
        if (savedInstanceState == null) host.openIntent(intent)
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
        // Only when leaving for good: a recreated activity finds the app still running.
        if (isFinishing) host.close()
        super.onDestroy()
    }
}
