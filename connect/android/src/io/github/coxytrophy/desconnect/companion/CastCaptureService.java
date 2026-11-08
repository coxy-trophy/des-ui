// SPDX-License-Identifier: MIT
package io.github.coxytrophy.desconnect.companion;

import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.ServiceInfo;
import android.graphics.Bitmap;
import android.graphics.PixelFormat;
import android.hardware.display.DisplayManager;
import android.hardware.display.VirtualDisplay;
import android.media.Image;
import android.media.ImageReader;
import android.media.projection.MediaProjection;
import android.media.projection.MediaProjectionManager;
import android.os.Build;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.IBinder;
import android.util.Log;
import android.view.Display;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.util.Arrays;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;

/** Owns capture only while its explicitly authorized, pinned TLS cast remains leased. */
public final class CastCaptureService extends Service {
    private static final String START = "des.connect.cast.START";
    private static final String STOP = "des.connect.cast.STOP";
    private static final String SESSION = "session";
    private static final String CHANNEL = "cast";
    private static final int NOTIFICATION_ID = 8043;
    private static final int MAX_CAPTURE_AXIS = 768;
    private static final long FRAME_INTERVAL_MS = 200L;

    // The permission result is single-use and never leaves this process or survives its death.
    private static final AtomicReference<Grant> HANDOFF = new AtomicReference<>();
    private static final class Grant {
        final CastLink cast;
        final CastSession.Token token;
        Intent permission;
        final boolean control;
        Grant(CastLink cast, CastSession.Token token, Intent permission, boolean control) {
            this.cast = cast;
            this.token = token;
            this.permission = permission;
            this.control = control;
        }
    }

    static boolean launch(Activity activity, CastLink cast, CastSession.Token token, Intent permission, boolean control) {
        if (permission == null || !cast.canStart(token)) return false;
        Grant grant = new Grant(cast, token, permission, control);
        if (!HANDOFF.compareAndSet(null, grant)) return false;
        try {
            Intent request = new Intent(activity, CastCaptureService.class).setAction(START).putExtra(SESSION, token.id);
            if (activity.startForegroundService(request) != null) return true;
        } catch (RuntimeException failure) {
            Log.w("DESConnectPreview", "Could not start screen sharing", failure);
        }
        HANDOFF.compareAndSet(grant, null);
        return false;
    }

    private final AtomicBoolean stopping = new AtomicBoolean();
    private final JpegBuffer jpeg = new JpegBuffer();
    private Grant grant;
    private HandlerThread thread;
    private Handler worker;
    private boolean watchingScreen;
    private MediaProjection projection;
    private VirtualDisplay output;
    private ImageReader reader;
    private Bitmap bitmap;
    private ByteBuffer packed;
    // Pre-34 projection is always whole-display. On newer Android, capture
    // and physical-display bounds are independent, including during rotation.
    private final CastGeometry bounds = new CastGeometry(Build.VERSION.SDK_INT < 34);
    private int width, height;
    private long geometry;
    private long lastEncoded;
    private boolean frameScheduled;
    private final Runnable captureLater = () -> {
        frameScheduled = false;
        if (reader != null) imageAvailable(reader);
    };

    private final MediaProjection.Callback projectionEvents = new MediaProjection.Callback() {
        @Override public void onStop() { finishCast(); }
        @Override public void onCapturedContentResize(int w, int h) {
            if (stopping.get() || w < 1 || h < 1) return;
            CastGeometry.Shape current = CastLink.displayShape(CastCaptureService.this);
            if (current == null) {
                finishCast();
                return;
            }
            resize(bounds.capture(current, w, h));
        }
        @Override public void onCapturedContentVisibilityChanged(boolean visible) {
            // Do not keep the last frame when a selected app is hidden.
            if (!visible) finishCast();
        }
    };

    private final BroadcastReceiver screenOff = new BroadcastReceiver() {
        @Override public void onReceive(Context context, Intent intent) { finishCast(); }
    };

    @Override public IBinder onBind(Intent intent) { return null; }

    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && STOP.equals(intent.getAction())) {
            finishCast();
            return START_NOT_STICKY;
        }
        Grant offered = HANDOFF.get();
        if (offered == null || intent == null || !START.equals(intent.getAction())
                || !offered.token.id.equals(intent.getStringExtra(SESSION))
                || !HANDOFF.compareAndSet(offered, null)) {
            if (grant == null) stopSelf();
            return START_NOT_STICKY;
        }
        if (grant != null || stopping.get()) {
            offered.cast.stop(offered.token);
            return START_NOT_STICKY;
        }
        grant = offered;
        try {
            Notification notification = notification();
            if (Build.VERSION.SDK_INT >= 29)
                startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION);
            else
                startForeground(NOTIFICATION_ID, notification);
            IntentFilter screen = new IntentFilter(Intent.ACTION_SCREEN_OFF);
            if (Build.VERSION.SDK_INT >= 33) registerReceiver(screenOff, screen, Context.RECEIVER_NOT_EXPORTED);
            else registerReceiver(screenOff, screen);
            watchingScreen = true;
            thread = new HandlerThread("DES Connect screen capture");
            thread.start();
            worker = new Handler(thread.getLooper());
            worker.post(this::initialize);
        } catch (RuntimeException failure) {
            Log.w("DESConnectPreview", "Could not prepare screen sharing", failure);
            finishCast();
        }
        return START_NOT_STICKY;
    }

    private Notification notification() {
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel(CHANNEL, getString(R.string.cast_title),
                NotificationManager.IMPORTANCE_LOW));
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE);
        PendingIntent stop = PendingIntent.getService(this, 0, new Intent(this, CastCaptureService.class)
                .setAction(STOP), PendingIntent.FLAG_IMMUTABLE);
        return new Notification.Builder(this, CHANNEL).setSmallIcon(android.R.drawable.ic_menu_camera)
                .setContentTitle(getString(R.string.cast_notification))
                .setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true)
                .addAction(new Notification.Action.Builder(null, getString(R.string.cast_stop), stop).build())
                .build();
    }

    private void initialize() {
        if (stopping.get() || !grant.cast.canStart(grant.token)
                || grant.control && !CastControlService.available()) {
            finishCast();
            return;
        }
        try {
            MediaProjectionManager manager = getSystemService(MediaProjectionManager.class);
            Intent permission = grant.permission;
            grant.permission = null;
            projection = manager.getMediaProjection(Activity.RESULT_OK, permission);
            if (projection == null) throw new IllegalStateException("Projection consent unavailable");
            projection.registerCallback(projectionEvents, worker);
            CastGeometry.Shape current = CastLink.displayShape(this);
            if (current == null || !grant.cast.activate(grant.token)) {
                finishCast();
                return;
            }
            resize(bounds.display(current));
            if (!stopping.get()) worker.post(watchdog);
        } catch (RuntimeException failure) {
            Log.w("DESConnectPreview", "Screen sharing could not begin", failure);
            finishCast();
        }
    }

    private final Runnable watchdog = new Runnable() {
        @Override public void run() {
            if (stopping.get() || !grant.cast.capturing(grant.token)
                    || grant.control && !CastControlService.available()) {
                finishCast();
                return;
            }
            CastGeometry.Shape current = CastLink.displayShape(CastCaptureService.this);
            if (current == null) {
                finishCast();
                return;
            }
            resize(bounds.display(current));
            if (!stopping.get()) worker.postDelayed(this, 150);
        }
    };

    private void resize(CastGeometry.Target target) {
        if (target == null || stopping.get()) return;
        if (!grant.cast.capturing(grant.token)) {
            finishCast();
            return;
        }
        // Invalidate old frames and input before swapping Android surfaces.
        if (reader != null) geometry = grant.cast.geometry(grant.token, null, false);
        double scale = Math.min(1.0, MAX_CAPTURE_AXIS / (double) Math.max(target.width, target.height));
        int newWidth = Math.max(1, (int) Math.round(target.width * scale));
        int newHeight = Math.max(1, (int) Math.round(target.height * scale));
        ImageReader fresh = null;
        try {
            fresh = ImageReader.newInstance(newWidth, newHeight, PixelFormat.RGBA_8888, 2);
            int density = getResources().getDisplayMetrics().densityDpi;
            if (output == null) {
                output = projection.createVirtualDisplay("DES Connect Cast", newWidth, newHeight, density,
                        DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, fresh.getSurface(), null, worker);
                if (output == null) throw new IllegalStateException("Projection display unavailable");
            } else {
                output.setSurface(null);
                output.resize(newWidth, newHeight, density);
                output.setSurface(fresh.getSurface());
            }
            releaseReader();
            reader = fresh;
            fresh = null;
            width = newWidth;
            height = newHeight;
            bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888);
            reader.setOnImageAvailableListener(this::imageAvailable, worker);
            geometry = grant.cast.geometry(grant.token, target.display, grant.control && target.full);
            if (geometry == 0) finishCast();
        } catch (RuntimeException failure) {
            Log.w("DESConnectPreview", "Could not configure the cast display", failure);
            finishCast();
        } finally {
            if (fresh != null) fresh.close();
        }
    }

    private void imageAvailable(ImageReader source) {
        if (source != reader || stopping.get()) return;
        long tick = CastLink.now();
        if (tick - lastEncoded < FRAME_INTERVAL_MS) {
            // Leave the newest image queued until the next encoding slot, even
            // if this turns out to be the last screen change for a while.
            if (!frameScheduled) {
                frameScheduled = true;
                worker.postDelayed(captureLater, FRAME_INTERVAL_MS - (tick - lastEncoded));
            }
            return;
        }
        if (frameScheduled) {
            worker.removeCallbacks(captureLater);
            frameScheduled = false;
        }
        try (Image image = source.acquireLatestImage()) {
            if (image == null) return;
            if (!grant.cast.capturing(grant.token)) {
                finishCast();
                return;
            }
            lastEncoded = tick;
            Image.Plane plane = image.getPlanes()[0];
            if (plane.getPixelStride() != 4) return;
            int rowBytes = width * 4;
            ByteBuffer pixels = plane.getBuffer();
            int stride = plane.getRowStride();
            if (stride != rowBytes) {
                if (stride < rowBytes || pixels.remaining() < (height - 1) * stride + rowBytes) return;
                if (packed == null) packed = ByteBuffer.allocateDirect(rowBytes * height);
                packed.clear();
                int origin = pixels.position();
                for (int y = 0; y < height; y++) {
                    int start = origin + y * stride;
                    pixels.limit(start + rowBytes).position(start);
                    packed.put(pixels);
                }
                packed.flip();
                pixels = packed;
            }
            bitmap.copyPixelsFromBuffer(pixels);
            byte[] compressed = compress(bitmap);
            if (compressed != null) grant.cast.frame(grant.token, geometry, compressed);
        } catch (RuntimeException failure) {
            Log.w("DESConnectPreview", "Cast frame capture failed", failure);
            finishCast();
        }
    }

    private byte[] compress(Bitmap image) {
        for (int quality : new int[]{70, 50, 34, 22}) {
            jpeg.reset();
            if (image.compress(Bitmap.CompressFormat.JPEG, quality, jpeg) && !jpeg.overflow) return jpeg.bytes();
        }
        Bitmap smaller = Bitmap.createScaledBitmap(image, Math.max(1, width / 2), Math.max(1, height / 2), true);
        try {
            jpeg.reset();
            return smaller.compress(Bitmap.CompressFormat.JPEG, 45, jpeg) && !jpeg.overflow ? jpeg.bytes() : null;
        } finally {
            if (smaller != image) smaller.recycle();
        }
    }

    private static final class JpegBuffer extends OutputStream {
        private final byte[] bytes = new byte[CastSession.MAX_JPEG];
        private int length;
        private boolean overflow;

        @Override public void write(int one) {
            if (length < bytes.length) bytes[length++] = (byte) one;
            else overflow = true;
        }

        @Override public void write(byte[] data, int start, int count) {
            if (count <= bytes.length - length) {
                System.arraycopy(data, start, bytes, length, count);
                length += count;
            } else overflow = true;
        }

        void reset() { length = 0; overflow = false; }
        byte[] bytes() { return Arrays.copyOf(bytes, length); }
    }

    private void finishCast() {
        if (!stopping.compareAndSet(false, true)) return;
        if (grant != null) grant.cast.stop(grant.token);
        stopSelf();
    }

    @Override public void onDestroy() {
        stopping.set(true);
        if (grant != null) grant.cast.stop(grant.token);
        if (watchingScreen) unregisterReceiver(screenOff);
        if (worker != null) {
            worker.removeCallbacksAndMessages(null);
            worker.post(this::releaseProjection);
            thread.quitSafely();
        }
        stopForeground(STOP_FOREGROUND_REMOVE);
        super.onDestroy();
    }

    private void releaseProjection() {
        if (output != null) {
            output.release();
            output = null;
        }
        releaseReader();
        if (projection != null) {
            projection.unregisterCallback(projectionEvents);
            projection.stop();
            projection = null;
        }
    }

    private void releaseReader() {
        worker.removeCallbacks(captureLater);
        frameScheduled = false;
        if (reader != null) {
            reader.close();
            reader = null;
        }
        if (bitmap != null) {
            bitmap.recycle();
            bitmap = null;
        }
        packed = null;
    }
}
