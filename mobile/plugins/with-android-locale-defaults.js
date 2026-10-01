const { withStringsXml } = require("expo/config-plugins");
const english = require("../locales/en.json");

// Expo writes configured locales to Android resources too; Android needs a default for each key.
module.exports = function withAndroidLocaleDefaults(config) {
  return withStringsXml(config, (config) => {
    const strings = config.modResults.resources.string;
    for (const [name, value] of Object.entries(english)) {
      if (!strings.some((item) => item.$.name === name)) strings.push({ $: { name }, _: value });
    }
    return config;
  });
};
