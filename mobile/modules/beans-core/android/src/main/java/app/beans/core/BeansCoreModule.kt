package app.beans.core

import expo.modules.kotlin.functions.Coroutine
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.beans_mobile.Core
import uniffi.beans_mobile.EventListener

/// The Rust core behind the phone: started once with the app's folder and the phone's facts,
/// then one request at a time and a stream of events.
class BeansCoreModule : Module() {
  private var core: Core? = null
  private val updater by lazy { Updater(requireNotNull(appContext.reactContext)) { downloaded, total ->
    sendEvent("updateProgress", mapOf("downloaded" to downloaded, "total" to total))
  } }

  override fun definition() = ModuleDefinition {
    Name("BeansCore")

    Events("event", "updateProgress")
    Function("updateCapability") { updater.capability() }
    Function("updateResult") { updater.result() }
    AsyncFunction("checkUpdate").Coroutine { withContext(Dispatchers.IO) { updater.check() } }
    AsyncFunction("installUpdate").Coroutine { id: String ->
      val activity = requireNotNull(appContext.currentActivity)
      withContext(Dispatchers.IO) { updater.install(id, activity) }
    }
    Function("cancelUpdate") { updater.cancel() }

    Function("start") { home: String, name: String, os: String, osVersion: String, model: String ->
      if (core == null) {
        core = Core.start(home, name, os, osVersion, model, object : EventListener {
          override fun onEvent(json: String) {
            sendEvent("event", mapOf("json" to json))
          }
        })
      }
    }

    // Blocks until the core answers. On the IO pool, never the JS thread and never the
    // module's single queue thread: a request that waits (pair.accept, up to ten minutes)
    // must not hold every other request behind it.
    AsyncFunction("request").Coroutine { method: String, params: String ->
      val running = core ?: throw IllegalStateException("The Beans core has not been started")
      withContext(Dispatchers.IO) { running.request(method, params) }
    }

    Function("wake") {
      core?.wake()
    }

    // The chat on screen, or null. While the app is in front PushService posts nothing for
    // it, and whatever it posted for it has now been seen.
    Function("setOpenChat") { chatId: String? ->
      PushService.openChat = chatId
      if (chatId != null && PushService.inFront) clearPosted(chatId)
    }

    OnActivityEntersForeground {
      PushService.inFront = true
      PushService.openChat?.let { clearPosted(it) }
    }

    OnActivityEntersBackground {
      PushService.inFront = false
      updater.cancel()
    }
  }

  private fun clearPosted(chatId: String) {
    appContext.reactContext?.let { PushService.clear(it, chatId) }
  }
}
