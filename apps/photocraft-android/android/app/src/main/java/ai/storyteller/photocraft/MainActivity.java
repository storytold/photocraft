package ai.storyteller.photocraft;

import com.google.androidgamesdk.GameActivity;

import android.content.Intent;
import android.os.Bundle;

/**
 * The Android entry point: a GameActivity (not a NativeActivity) because eframe/winit need the
 * GameActivity surface and input queue, and because accesskit + native-activity is a hard eframe
 * compile error.
 *
 * <p>The Rust cdylib is loaded here ({@code android.app.lib_name=main} in the manifest) and
 * {@code android_main} runs on GameActivity's own thread.
 */
public class MainActivity extends GameActivity {

    static {
        // 加载 Rust cdylib（对应 AndroidManifest 里的 android.app.lib_name=main）
        System.loadLibrary("main");
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == SafBridge.REQUEST_CODE || requestCode == SafBridge.REQUEST_SAVE) {
            SafBridge.onResult(this, requestCode, resultCode == RESULT_OK ? data : null);
        }
    }

    /**
     * Rust calls these instance methods (calling a method on an object we already hold needs no
     * class lookup, which a natively attached thread cannot do for app classes).
     */
    public void safPickMedia() {
        SafBridge.pickMedia(this);
    }

    /** Whether the import picker and the copy into the app directory are finished. */
    public boolean safIsDone() {
        return SafBridge.isDone();
    }

    /** Open the "where should this be saved?" picker. */
    public void safPickSave(String title) {
        SafBridge.pickSave(this, title);
    }

    public boolean safSaveDone() {
        return SafBridge.isSaveDone();
    }

    public boolean safHasSaveUri() {
        return SafBridge.hasSaveUri();
    }

    /** Copy a finished file to the destination chosen by {@link #safPickSave}. */
    public boolean safPublish(String path) {
        return SafBridge.publish(this, path);
    }

    /** Write a saved document back to the document it was imported from. */
    public boolean safPublishTo(String uri, String path) {
        return SafBridge.publishTo(this, uri, path);
    }
}
