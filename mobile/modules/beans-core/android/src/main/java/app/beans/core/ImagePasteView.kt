package app.beans.core

import android.content.Context
import android.graphics.BitmapFactory
import android.media.ExifInterface
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.system.Os
import android.system.OsConstants
import android.view.View
import android.webkit.MimeTypeMap
import android.widget.EditText
import androidx.core.view.ContentInfoCompat
import androidx.core.view.OnReceiveContentListener
import androidx.core.view.ViewCompat
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import java.io.File
import java.util.UUID
import java.util.concurrent.Executors

/// A container that takes images put into the text input inside it: pasted from the clipboard,
/// inserted by the keyboard, or dropped on it. Each image is copied to a file and the files go
/// to `onPasteImages`; text that came with them lands in the field as usual.
class ImagePasteViewModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("ImagePasteView")
    OnCreate {
      appContext.reactContext?.applicationContext?.let { context ->
        PasteImageCache.copier.execute { PasteImageCache.pruneOnce(context) }
      }
    }
    AsyncFunction("deleteFiles") { uris: List<String> ->
      appContext.reactContext?.applicationContext?.let { context ->
        PasteImageCache.copier.submit { PasteImageCache.deleteFiles(context, uris) }.get()
      }
      Unit
    }
    View(ImagePasteView::class) {
      Events("onPasteImages")
    }
  }
}

/// Process-wide serialization keeps startup pruning ahead of writes, including module reloads.
private object PasteImageCache {
  val copier = Executors.newSingleThreadExecutor()
  private var pruned = false
  private val namePattern = Regex("[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\\.[a-zA-Z0-9]+")

  fun directory(context: Context) = File(context.cacheDir.canonicalFile, "Pasted")

  fun safeDirectory(directory: File): Boolean = runCatching {
    OsConstants.S_ISDIR(Os.lstat(directory.path).st_mode) && directory == directory.canonicalFile
  }.getOrDefault(false)

  private fun ownedFile(context: Context, value: String): File? = runCatching {
    val uri = Uri.parse(value)
    if (uri.scheme != "file" || !uri.authority.isNullOrEmpty() || uri.query != null || uri.fragment != null) return@runCatching null
    val directory = directory(context)
    val file = File(uri.path ?: return@runCatching null)
    if (!safeDirectory(directory) || file.parentFile != directory || file != file.canonicalFile ||
      !namePattern.matches(file.name) ||
      MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase())?.startsWith("image/") != true ||
      !OsConstants.S_ISREG(Os.lstat(file.path).st_mode)) return@runCatching null
    file
  }.getOrNull()

  fun deleteFiles(context: Context, uris: List<String>) {
    uris.forEach { ownedFile(context, it)?.delete() }
  }

  fun pruneOnce(context: Context) {
    if (pruned) return
    pruned = true
    val directory = directory(context)
    if (!safeDirectory(directory)) return
    deleteFiles(context, directory.listFiles()?.map { Uri.fromFile(it).toString() } ?: emptyList())
  }
}

class ImagePasteView(context: Context, appContext: AppContext) : ExpoView(context, appContext) {
  private val onPasteImages by EventDispatcher()
  private var attachmentGeneration = 0
  private val mainHandler = Handler(Looper.getMainLooper())

  override fun onDetachedFromWindow() {
    attachmentGeneration += 1
    super.onDetachedFromWindow()
  }

  /// Called for every paste, keyboard insertion, and drop; what is not an image goes back to
  /// the field's own handling.
  private val receiver = OnReceiveContentListener { view, payload ->
    val split = payload.partition { item -> item.uri?.let { mimeOf(view.context, it, payload) } != null }
    split.first?.let { images ->
      val uris = (0 until images.clip.itemCount).mapNotNull { images.clip.getItemAt(it).uri }
      receive(view.context, uris, payload)
    }
    split.second
  }

  override fun onViewAdded(child: View) {
    super.onViewAdded(child)
    if (child is EditText) ViewCompat.setOnReceiveContentListener(child, MIME_TYPES, receiver)
  }

  override fun onViewRemoved(child: View) {
    if (child is EditText) ViewCompat.setOnReceiveContentListener(child, null, null)
    super.onViewRemoved(child)
  }

  private fun receive(context: Context, uris: List<Uri>, payload: ContentInfoCompat) {
    val mimes = uris.map { mimeOf(context, it, payload) ?: "image/png" }
    val generation = attachmentGeneration
    PasteImageCache.copier.execute {
      PasteImageCache.pruneOnce(context)
      val files = uris.zip(mimes).mapNotNull { (uri, mime) -> runCatching { copy(context, uri, mime) }.getOrNull() }
      if (files.isNotEmpty()) {
        // View.post queues callbacks until reattachment; the main handler also cleans up
        // results whose view detached while a provider was still copying bytes.
        mainHandler.post {
          if (isAttachedToWindow && attachmentGeneration == generation) {
            onPasteImages(mapOf("files" to files))
          } else {
            PasteImageCache.copier.execute {
              PasteImageCache.deleteFiles(context, files.mapNotNull { it["uri"] as? String })
            }
          }
        }
      }
    }
  }

  private fun copy(context: Context, uri: Uri, mime: String): Map<String, Any> {
    val directory = PasteImageCache.directory(context).apply { mkdirs() }
    check(PasteImageCache.safeDirectory(directory)) { "Invalid paste cache directory" }
    val extension = MimeTypeMap.getSingleton().getExtensionFromMimeType(mime) ?: "png"
    val file = File(directory, "${UUID.randomUUID()}.$extension")
    var created = false
    try {
      val input = context.contentResolver.openInputStream(uri) ?: throw IllegalStateException("Cannot open $uri")
      input.use { source ->
        check(file.createNewFile()) { "Paste cache file already exists" }
        created = true
        file.outputStream().use { source.copyTo(it) }
      }
      val result = mutableMapOf<String, Any>("uri" to Uri.fromFile(file).toString(), "mime" to mime, "size" to file.length().toDouble())
      pixelSize(file)?.let { (width, height) ->
        result["width"] = width
        result["height"] = height
      }
      return result
    } catch (error: Exception) {
      if (created) PasteImageCache.deleteFiles(context, listOf(Uri.fromFile(file).toString()))
      throw error
    }
  }

  /// The size as shown: an EXIF orientation of 5 through 8 turns the image on its side.
  private fun pixelSize(file: File): Pair<Int, Int>? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(file.path, bounds)
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    val orientation = runCatching { ExifInterface(file.path).getAttributeInt(ExifInterface.TAG_ORIENTATION, 1) }.getOrDefault(1)
    return if (orientation >= 5) bounds.outHeight to bounds.outWidth else bounds.outWidth to bounds.outHeight
  }

  private companion object {
    val MIME_TYPES = arrayOf("image/*")

    /// The item's image type, from its provider or else from the clip's description.
    fun mimeOf(context: Context, uri: Uri, payload: ContentInfoCompat): String? {
      context.contentResolver.getType(uri)?.let { return if (it.startsWith("image/")) it else null }
      val description = payload.clip.description
      return (0 until description.mimeTypeCount).map { description.getMimeType(it) }.firstOrNull { it.startsWith("image/") }
    }
  }
}
