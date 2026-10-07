const { withAndroidManifest } = require("expo/config-plugins");
const { readFileSync, statSync } = require("node:fs");
const { resolve } = require("node:path");

// Runs only for Android prebuild; iOS does not need Firebase client configuration.
module.exports = function withAndroidPushConfig(config) {
  return withAndroidManifest(config, (config) => {
    const supplied = process.env.BEANS_GOOGLE_SERVICES_FILE;
    if (!supplied || config.android?.googleServicesFile !== supplied) {
      throw new Error("Android push builds require BEANS_GOOGLE_SERVICES_FILE from your existing Firebase project");
    }
    const path = resolve(config.modRequest.projectRoot, supplied);
    if (!statSync(path).isFile()) throw new Error("BEANS_GOOGLE_SERVICES_FILE must name an existing Firebase client JSON file");
    const client = JSON.parse(readFileSync(path, "utf8"));
    if (!client.project_info?.project_id || !client.project_info?.project_number ||
        !client.client?.some((entry) => entry.client_info?.android_client_info?.package_name === config.android.package)) {
      throw new Error(`BEANS_GOOGLE_SERVICES_FILE must identify an existing Firebase project and Android client ${config.android.package}`);
    }
    return config;
  });
};
