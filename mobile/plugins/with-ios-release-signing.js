const { withXcodeProject } = require("expo/config-plugins");

const required = ["BEANS_APPLE_TEAM_ID", "BEANS_IOS_APP_PROFILE_NAME", "BEANS_IOS_NOTIFY_PROFILE_NAME"];

module.exports = function withIOSReleaseSigning(config) {
  const profileConfigured = ["BEANS_IOS_APP_PROFILE_NAME", "BEANS_IOS_NOTIFY_PROFILE_NAME"].some((name) => process.env[name]);
  if (!profileConfigured) return config;
  const supplied = required.filter((name) => process.env[name]);
  if (supplied.length !== required.length) throw new Error("with-ios-release-signing: incomplete provisioning configuration");
  const team = process.env.BEANS_APPLE_TEAM_ID;
  if (!/^[A-Z0-9]{10}$/.test(team)) throw new Error("with-ios-release-signing: invalid Apple team");
  const identity = process.env.BEANS_IOS_SIGN_IDENTITY || "Apple Distribution";
  return withXcodeProject(config, (config) => {
    const project = config.modResults;
    const lists = project.pbxXCConfigurationList();
    const configurations = project.pbxXCBuildConfigurationSection();
    let appConfigured = false;
    let notificationConfigured = false;
    for (const target of Object.values(project.pbxNativeTargetSection())) {
      if (!target || typeof target !== "object") continue;
      const productType = String(target.productType || "").replaceAll('"', "");
      const targetName = String(target.name || "").replaceAll('"', "");
      const isApp = productType === "com.apple.product-type.application" && targetName === config.name;
      const isNotification = productType === "com.apple.product-type.app-extension" && targetName === "LorcaNotify";
      if (!isApp && !isNotification) continue;
      const profile = isApp ? process.env.BEANS_IOS_APP_PROFILE_NAME : process.env.BEANS_IOS_NOTIFY_PROFILE_NAME;
      const list = lists[target.buildConfigurationList];
      if (!list) throw new Error("with-ios-release-signing: target has no build configurations");
      for (const reference of list.buildConfigurations) {
        const build = configurations[reference.value];
        if (!build || String(build.name).replaceAll('"', "") !== "Release") continue;
        Object.assign(build.buildSettings, {
          CODE_SIGN_STYLE: "Manual",
          CODE_SIGN_IDENTITY: JSON.stringify(identity),
          DEVELOPMENT_TEAM: team,
          PROVISIONING_PROFILE_SPECIFIER: JSON.stringify(profile),
        });
        if (productType === "com.apple.product-type.application") appConfigured = true;
        else notificationConfigured = true;
      }
    }
    if (!appConfigured || !notificationConfigured) throw new Error("with-ios-release-signing: app and notification Release targets are required");
    return config;
  });
};
