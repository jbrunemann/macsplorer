package dev.macsplorer.ai;

import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.net.Uri;

import java.io.File;

/** What Rust calls to open a file, or a web address, in the app the system picks for it. */
public final class Opener {
    private Opener() {}

    /**
     * Starts {@code ACTION_VIEW} for the file with read access to it granted through {@link Files}.
     *
     * @return {@code null} when an app took it, otherwise what to tell the user
     */
    public static String open(Activity activity, String path) {
        if (path.startsWith("http://") || path.startsWith("https://")) {
            return start(activity, new Intent(Intent.ACTION_VIEW, Uri.parse(path)));
        }
        File file = new File(path);
        if (!file.isFile()) {
            return "the file is gone";
        }
        String mime = Files.mimeOf(file.getName());
        Intent intent = new Intent(Intent.ACTION_VIEW);
        intent.setDataAndType(Files.uriFor(file), mime != null ? mime : "*/*");
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        return start(activity, intent);
    }

    private static String start(Activity activity, Intent intent) {
        try {
            activity.startActivity(intent);
            return null;
        } catch (ActivityNotFoundException e) {
            return "no app can open this type of file";
        } catch (RuntimeException e) {
            return String.valueOf(e.getMessage());
        }
    }
}
