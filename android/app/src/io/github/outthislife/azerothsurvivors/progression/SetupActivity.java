package io.github.outthislife.azerothsurvivors.progression;

import android.app.Activity;
import android.content.Intent;
import android.database.Cursor;
import android.graphics.Color;
import android.graphics.Typeface;
import android.net.Uri;
import android.os.Bundle;
import android.provider.DocumentsContract;
import android.provider.DocumentsContract.Document;
import android.view.Gravity;
import android.view.WindowManager;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.BufferedInputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.util.Arrays;
import java.util.HashSet;
import java.util.Locale;
import java.util.Set;
import java.util.TreeSet;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;

public final class SetupActivity extends Activity {
    private static final int PICK_FOLDER = 1;
    private static final int PICK_ZIP = 2;
    private static final long MAX_DATA_BYTES = 16L * 1024 * 1024 * 1024;
    private static final Set<String> REQUIRED = new HashSet<>(Arrays.asList(
        "dbc.mpq", "speech.mpq", "fonts.mpq", "interface.mpq", "misc.mpq", "sound.mpq",
        "wmo.mpq", "terrain.mpq", "texture.mpq", "model.mpq", "patch.mpq"));
    private static final String[] COLUMNS = {
        Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE
    };

    private final ExecutorService worker = Executors.newSingleThreadExecutor();
    private File data;
    private File staging;
    private File previous;
    private Button play;
    private Button folder;
    private Button zip;
    private TextView status;
    private boolean importing;
    private long copiedBytes;

    @Override
    public void onCreate(Bundle state) {
        super.onCreate(state);
        data = new File(getFilesDir(), "Data");
        staging = new File(getFilesDir(), "Data.import");
        previous = new File(getFilesDir(), "Data.previous");
        if (!data.exists() && previous.exists()) previous.renameTo(data);

        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setBackgroundColor(Color.rgb(23, 18, 16));
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setGravity(Gravity.CENTER);
        layout.setPadding(dp(24), dp(24), dp(24), dp(24));
        scroll.addView(layout);

        TextView title = text("Azeroth Survivors", 32, Color.rgb(239, 200, 126));
        title.setTypeface(Typeface.SERIF, Typeface.BOLD);
        layout.addView(title);
        TextView detail = text("Import an English WoW 1.12.1 (build 5875) Data folder or client ZIP. Allow about 5.4 GB for the imported files.", 16, Color.rgb(225, 214, 200));
        detail.setMaxWidth(dp(480));
        detail.setPadding(0, dp(16), 0, dp(16));
        layout.addView(detail);
        play = addButton(layout, "Play", () -> startActivity(new Intent(this, GameActivity.class)));
        folder = addButton(layout, "Choose Data folder", () ->
            startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE), PICK_FOLDER));
        zip = addButton(layout, "Choose client ZIP", () -> {
            Intent picker = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                .addCategory(Intent.CATEGORY_OPENABLE).setType("*/*");
            startActivityForResult(picker, PICK_ZIP);
        });
        status = text("", 14, Color.rgb(239, 200, 126));
        status.setPadding(0, dp(15), 0, 0);
        layout.addView(status);
        setContentView(scroll);
        showStatus(hasData() ? "Game data ready" : "Game data is required to play");
    }

    private TextView text(String label, int size, int color) {
        TextView view = new TextView(this);
        view.setText(label);
        view.setTextSize(size);
        view.setTextColor(color);
        view.setGravity(Gravity.CENTER);
        return view;
    }

    private Button addButton(LinearLayout layout, String label, Runnable action) {
        Button button = new Button(this);
        button.setText(label);
        button.setMinWidth(dp(235));
        button.setMinHeight(dp(48));
        button.setOnClickListener(view -> action.run());
        layout.addView(button);
        return button;
    }

    private int dp(int size) {
        return Math.round(size * getResources().getDisplayMetrics().density);
    }

    private boolean hasData() {
        return REQUIRED.stream().allMatch(name -> new File(data, name).isFile());
    }

    private void showStatus(String message) {
        status.setText(message);
        play.setEnabled(!importing && hasData());
        folder.setEnabled(!importing);
        zip.setEnabled(!importing);
    }

    @Override
    protected void onActivityResult(int request, int result, Intent intent) {
        super.onActivityResult(request, result, intent);
        if (result != RESULT_OK || intent == null || intent.getData() == null || importing) return;
        if (request != PICK_FOLDER && request != PICK_ZIP) return;
        Uri uri = intent.getData();
        importing = true;
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        showStatus("Importing game data…");
        worker.execute(() -> {
            String message;
            try {
                importData(uri, request);
                message = "Game data ready. Tap Play.";
            } catch (IOException | RuntimeException error) {
                message = "Import failed. " + error.getMessage();
            }
            String resultMessage = message;
            runOnUiThread(() -> {
                if (isDestroyed()) return;
                importing = false;
                getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                showStatus(resultMessage);
            });
        });
    }

    private void importData(Uri uri, int request) throws IOException {
        remove(staging);
        if (!staging.mkdir()) throw new IOException("Could not prepare app storage.");
        copiedBytes = 0;
        Set<String> copied = new HashSet<>();
        try {
            if (request == PICK_FOLDER) copyFolder(uri, copied);
            else copyZip(uri, copied);
            Set<String> missing = new TreeSet<>(REQUIRED);
            missing.removeAll(copied);
            if (!missing.isEmpty()) throw new IOException("Missing " + String.join(", ", missing) + ".");
            remove(previous);
            if (data.exists() && !data.renameTo(previous)) throw new IOException("Could not keep the previous import.");
            if (!staging.renameTo(data)) {
                if (previous.exists()) previous.renameTo(data);
                throw new IOException("Could not finish writing the data folder.");
            }
            remove(previous);
        } finally {
            remove(staging);
        }
    }

    private void copyFolder(Uri tree, Set<String> copied) throws IOException {
        String root = DocumentsContract.getTreeDocumentId(tree);
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, root);
        try (Cursor cursor = getContentResolver().query(children, COLUMNS, null, null, null)) {
            if (cursor == null) throw new IOException("Could not open the selected folder.");
            while (cursor.moveToNext()) {
                if (Document.MIME_TYPE_DIR.equals(cursor.getString(2))) continue;
                String name = archiveName(cursor.getString(1));
                if (name == null) continue;
                Uri document = DocumentsContract.buildDocumentUriUsingTree(tree, cursor.getString(0));
                try (InputStream stream = open(document)) {
                    copyArchive(stream, name, copied);
                }
            }
        }
    }

    private void copyZip(Uri uri, Set<String> copied) throws IOException {
        try (ZipInputStream stream = new ZipInputStream(new BufferedInputStream(open(uri)))) {
            ZipEntry entry;
            String dataParent = null;
            while ((entry = stream.getNextEntry()) != null) {
                String path = entry.getName().replace('\\', '/');
                int slash = path.lastIndexOf('/');
                String parent = slash < 0 ? "" : path.substring(0, slash);
                String name = archiveName(path.substring(slash + 1));
                if (!entry.isDirectory() && name != null
                    && (parent.isEmpty() || parent.equalsIgnoreCase("Data")
                        || parent.toLowerCase(Locale.ROOT).endsWith("/data"))) {
                    if (dataParent == null) dataParent = parent;
                    if (dataParent.equalsIgnoreCase(parent)) copyArchive(stream, name, copied);
                }
                stream.closeEntry();
            }
        }
    }

    private InputStream open(Uri uri) throws IOException {
        InputStream stream = getContentResolver().openInputStream(uri);
        if (stream == null) throw new IOException("Could not read the selected files.");
        return stream;
    }

    private static String archiveName(String candidate) {
        if (candidate == null) return null;
        String name = candidate.toLowerCase(Locale.ROOT);
        return REQUIRED.contains(name) || name.equals("speech2.mpq")
            || name.matches("patch-[a-z0-9]\\.mpq") ? name : null;
    }

    private void copyArchive(InputStream stream, String name, Set<String> copied) throws IOException {
        if (copied.contains(name)) throw new IOException("Duplicate archive " + name + ".");
        runOnUiThread(() -> {
            if (!isDestroyed()) showStatus("Importing " + name + "…");
        });
        byte[] header = new byte[4];
        int count = 0;
        while (count < header.length) {
            int read = stream.read(header, count, header.length - count);
            if (read < 0) break;
            count += read;
        }
        if (count < header.length || header[0] != 'M' || header[1] != 'P' || header[2] != 'Q'
            || (header[3] != 0x1a && header[3] != 0x1b)) {
            throw new IOException(name + " does not have an MPQ header.");
        }
        byte[] buffer = new byte[256 * 1024];
        try (FileOutputStream out = new FileOutputStream(new File(staging, name))) {
            out.write(header);
            copiedBytes += header.length;
            for (int read; (read = stream.read(buffer)) != -1; ) {
                copiedBytes += read;
                if (copiedBytes > MAX_DATA_BYTES) throw new IOException("Game data exceeds 16 GB.");
                out.write(buffer, 0, read);
            }
        }
        copied.add(name);
    }

    private static void remove(File path) throws IOException {
        if (!path.exists()) return;
        File[] children = path.listFiles();
        if (children != null) {
            for (File child : children) remove(child);
        }
        if (!path.delete()) throw new IOException("Could not clear app storage.");
    }

    @Override
    protected void onDestroy() {
        worker.shutdown();
        super.onDestroy();
    }
}
