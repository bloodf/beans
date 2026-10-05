const path = require("node:path");
const { getDefaultConfig } = require("expo/metro-config");

const config = getDefaultConfig(__dirname);

// Metro resolves the file-linked Blobatar source outside mobile's module tree.
config.watchFolders.push(path.resolve(__dirname, "../packages/beans-blobatar"));
config.resolver.nodeModulesPaths.push(path.resolve(__dirname, "node_modules"));

module.exports = config;
