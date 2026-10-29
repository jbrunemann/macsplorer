package dev.macsplorer.ai;

import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.net.Uri;
import android.provider.DocumentsContract;

import java.io.File;

/** What Rust calls to open a file or a web address in the app the system picks, or to show a folder in the Files app. */
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

    /**
     * Shows the folder that holds {@code path} in the system's Files app. The app can't be told to select the item.
     * Only shared storage and memory cards have a document address there.
     *
     * @return {@code null} when an app took it, otherwise what to tell the user
     */
    public static String reveal(Activity activity, String path) {
        File folder = new File(path).getParentFile();
        String id = folder == null ? null : documentId(folder.getAbsolutePath());
        if (id == null) {
            return "the file manager can't show this folder";
        }
        Intent intent = new Intent(Intent.ACTION_VIEW);
        intent.setDataAndType(DocumentsContract.buildDocumentUri("com.android.externalstorage.documents", id), DocumentsContract.Document.MIME_TYPE_DIR);
        return start(activity, intent);
    }

    /** {@code primary:Download} for {@code /storage/emulated/0/Download}, {@code 1234-ABCD:DCIM} for a card; {@code null} elsewhere. */
    static String documentId(String path) {
        for (String prefix : new String[] {"/storage/emulated/0", "/sdcard"}) {
            if (path.equals(prefix) || path.startsWith(prefix + "/")) {
                return "primary:" + path.substring(prefix.length()).replaceFirst("^/", "");
            }
        }
        if (path.startsWith("/storage/")) {
            String rest = path.substring("/storage/".length());
            int slash = rest.indexOf('/');
            String volume = slash < 0 ? rest : rest.substring(0, slash);
            if (!volume.isEmpty() && !volume.equals("emulated") && !volume.equals("self")) {
                return volume + ":" + (slash < 0 ? "" : rest.substring(slash + 1));
            }
        }
        return null;
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
