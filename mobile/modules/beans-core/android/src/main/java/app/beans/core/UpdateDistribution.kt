package app.beans.core

internal object UpdateDistribution {
  // A GitHub-profile APK still needs an eligible source. A missing installer is a sideload;
  // our own installer is retained after a successful Beans update. Every other installer,
  // including Play and third-party stores, stays outside the self-update channel.
  fun allowsInstaller(installer: String?, applicationId: String): Boolean =
    installer == null || installer == applicationId
}
