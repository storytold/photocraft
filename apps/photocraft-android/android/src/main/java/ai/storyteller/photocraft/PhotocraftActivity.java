package ai.storyteller.photocraft;

import android.app.NativeActivity;
import android.os.Bundle;
import android.view.MotionEvent;

/**
 * Keeps Android's native input queue intact (winit handles pointer locations),
 * and sends additional pen pressure, tilt, and button samples to Rust.
 *
 * No touch is consumed here: super.dispatchTouchEvent forwards the original
 * event, so PhotoCraft receives exactly one pointer stream.
 *
 * Depending on how NativeActivity routes device input, dispatchTouchEvent may
 * not receive every native-queue event. In that case the winit pressure path
 * remains available, but tilt/button data require a deeper input hook.
 */
public final class PhotocraftActivity extends NativeActivity {
    private boolean penDown;

    // JNI symbol is implemented in src/spen.rs in libphotocraft_android.so.
    private native void nativeStylusSample(
            float pressure,
            float tiltRadians,
            float orientationRadians,
            boolean eraser,
            boolean contact
    );

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        penDown = false;
    }

    @Override
    public boolean dispatchTouchEvent(MotionEvent event) {
        forwardStylus(event);
        return super.dispatchTouchEvent(event);
    }

    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent event) {
        forwardStylus(event);
        return super.dispatchGenericMotionEvent(event);
    }

    @Override
    protected void onPause() {
        endPen();
        super.onPause();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        if (!hasFocus) {
            endPen();
        }
        super.onWindowFocusChanged(hasFocus);
    }

    private void endPen() {
        if (penDown) {
            nativeStylusSample(0.0f, 0.0f, 0.0f, false, false);
            penDown = false;
        }
    }

    private static int stylusIndex(MotionEvent e) {
        for (int i = 0; i < e.getPointerCount(); i++) {
            int type = e.getToolType(i);
            if (type == MotionEvent.TOOL_TYPE_STYLUS
                    || type == MotionEvent.TOOL_TYPE_ERASER) {
                return i;
            }
        }
        return -1;
    }

    private void emit(MotionEvent event, int pointer, int historyIndex) {
        int toolType = event.getToolType(pointer);
        boolean eraser = toolType == MotionEvent.TOOL_TYPE_ERASER
                || (event.getButtonState() & MotionEvent.BUTTON_STYLUS_PRIMARY) != 0;
        float pressure;
        float tilt;
        float orientation;

        if (historyIndex < 0) {
            pressure = event.getAxisValue(MotionEvent.AXIS_PRESSURE, pointer);
            tilt = event.getAxisValue(MotionEvent.AXIS_TILT, pointer);
            orientation = event.getAxisValue(MotionEvent.AXIS_ORIENTATION, pointer);
        } else {
            pressure = event.getHistoricalAxisValue(MotionEvent.AXIS_PRESSURE, pointer, historyIndex);
            tilt = event.getHistoricalAxisValue(MotionEvent.AXIS_TILT, pointer, historyIndex);
            orientation = event.getHistoricalAxisValue(MotionEvent.AXIS_ORIENTATION, pointer, historyIndex);
        }
        nativeStylusSample(pressure, tilt, orientation, eraser, true);
        penDown = true;
    }

    private void forwardStylus(MotionEvent event) {
        int action = event.getActionMasked();

        if (action == MotionEvent.ACTION_CANCEL
                || action == MotionEvent.ACTION_UP
                || action == MotionEvent.ACTION_HOVER_EXIT) {
            int pen = stylusIndex(event);
            if (pen >= 0 && action == MotionEvent.ACTION_UP) {
                emit(event, pen, -1);
            }
            endPen();
            return;
        }

        // Hover events must not start painting.
        if (action == MotionEvent.ACTION_HOVER_ENTER
                || action == MotionEvent.ACTION_HOVER_MOVE) {
            endPen();
            return;
        }

        int pen = stylusIndex(event);
        if (pen < 0) {
            return; // Finger contact is processed by winit, not by the pen bridge.
        }

        if (action == MotionEvent.ACTION_POINTER_UP
                && event.getActionIndex() == pen) {
            emit(event, pen, -1);
            endPen();
            return;
        }

        if (action != MotionEvent.ACTION_DOWN
                && action != MotionEvent.ACTION_POINTER_DOWN
                && action != MotionEvent.ACTION_MOVE) {
            return;
        }

        // Preserve high-frequency S Pen pressure samples coalesced by Android.
        if (action == MotionEvent.ACTION_MOVE) {
            for (int h = 0; h < event.getHistorySize(); h++) {
                emit(event, pen, h);
            }
        }
        emit(event, pen, -1);
    }
}
