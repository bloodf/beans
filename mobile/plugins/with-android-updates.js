const { withAndroidManifest } = require("expo/config-plugins");

module.exports = (config, { enabled = false } = {}) => withAndroidManifest(config, (config) => {
  const manifest = config.modResults.manifest;
  const permission = "android.permission.REQUEST_INSTALL_PACKAGES";
  manifest["uses-permission"] = (manifest["uses-permission"] || []).filter(item => item.$["android:name"] !== permission);
  if (enabled) manifest["uses-permission"].push({ $: { "android:name": permission } });
  const app = manifest.application[0];
  app["meta-data"] = (app["meta-data"] || []).filter(item => item.$["android:name"] !== "app.beans.GITHUB_UPDATES");
  app["meta-data"].push({ $: { "android:name": "app.beans.GITHUB_UPDATES", "android:value": String(enabled) } });
  return config;
});
