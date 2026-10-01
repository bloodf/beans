const { withAppBuildGradle } = require("expo/config-plugins");

const names = ["BEANS_ANDROID_KEYSTORE", "BEANS_ANDROID_KEYSTORE_PASSWORD", "BEANS_ANDROID_KEY_ALIAS", "BEANS_ANDROID_KEY_PASSWORD"];

module.exports = function withAndroidReleaseSigning(config) {
  const supplied = names.filter((name) => process.env[name]);
  if (supplied.length === 0) return config;
  if (supplied.length !== names.length) throw new Error("with-android-release-signing: incomplete signing environment");
  return withAppBuildGradle(config, (config) => {
    const source = config.modResults.contents;
    const debugConfig = "    signingConfigs {\n        debug {";
    const debugRelease = "            signingConfig signingConfigs.debug\n            def enableShrinkResources";
    if (!source.includes(debugConfig) || !source.includes(debugRelease)) {
      throw new Error("with-android-release-signing: the Gradle template changed; update the plugin");
    }
    config.modResults.contents = source
      .replace(debugConfig, `    signingConfigs {
        release {
            storeFile file(System.getenv('BEANS_ANDROID_KEYSTORE'))
            storePassword System.getenv('BEANS_ANDROID_KEYSTORE_PASSWORD')
            keyAlias System.getenv('BEANS_ANDROID_KEY_ALIAS')
            keyPassword System.getenv('BEANS_ANDROID_KEY_PASSWORD')
        }
        debug {`)
      .replace(debugRelease, `            signingConfig signingConfigs.release
            def enableShrinkResources`);
    return config;
  });
};
