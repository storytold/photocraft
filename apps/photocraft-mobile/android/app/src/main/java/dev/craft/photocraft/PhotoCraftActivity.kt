package dev.craft.photocraft

import android.app.NativeActivity
import android.os.Bundle

/**
 * The whole Android shell. NativeActivity loads libphotocraft_mobile.so (named in the manifest)
 * and android-activity's glue calls its `android_main` on a thread of its own. Native features
 * (the Storage Access Framework picker, stylus details) attach here later.
 */
class PhotoCraftActivity : NativeActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        System.loadLibrary("photocraft_mobile")
        super.onCreate(savedInstanceState)
    }
}
