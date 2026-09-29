package rocks.spotifast.spotifast;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.os.IBinder;

/**
 * Keeps the process unfrozen while Spotify signs in.
 *
 * <p>Approving the sign-in backgrounds the app behind the browser, and the OS
 * promptly freezes the process, so the loopback listener cannot receive
 * Spotify's redirect. This foreground service (with its notification) keeps
 * the process alive until the sign-in flow stops it. Started and stopped from
 * Rust (src/auth_android.rs); only one sign-in flow runs at a time, so it is
 * never started twice at once.
 */
public class AuthKeepaliveService extends Service {
    private static final String CHANNEL_ID = "signin-keepalive";
    private static final int NOTIFICATION_ID = 1;

    @Override
    public void onCreate() {
        super.onCreate();
        NotificationManager notifications = getSystemService(NotificationManager.class);
        notifications.createNotificationChannel(
                new NotificationChannel(CHANNEL_ID, "Sign-in", NotificationManager.IMPORTANCE_LOW));
        PendingIntent tap = null;
        Intent launch = getPackageManager().getLaunchIntentForPackage(getPackageName());
        if (launch != null) {
            tap = PendingIntent.getActivity(this, 0, launch, PendingIntent.FLAG_IMMUTABLE);
        }
        int icon = getResources().getIdentifier("icon", "mipmap", getPackageName());
        Notification notification =
                new Notification.Builder(this, CHANNEL_ID)
                        .setContentTitle("Waiting for Spotify sign-in")
                        .setContentText("Tap to return to Spotifast")
                        .setContentIntent(tap)
                        .setSmallIcon(icon != 0 ? icon : android.R.drawable.ic_dialog_info)
                        .build();
        startForeground(NOTIFICATION_ID, notification);
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        return START_NOT_STICKY;
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
