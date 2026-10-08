import { expect, test } from "bun:test";
import { browserCLIPort, parseCLIPort, requireCLIPort } from "./cliPort";

test("stable and Dev accept only their matching isolated port", () => {
  for (const expected of [4874, 4875]) {
    expect(requireCLIPort(expected, expected)).toBe(expected);
    expect(parseCLIPort(String(expected), expected)).toBe(expected);
    for (const text of [` ${expected}`, `+${expected}`, `0${expected}`, `${expected}.0`]) {
      expect(parseCLIPort(text, expected)).toBe(expected);
    }
    for (const value of [0, -1, 4864, expected === 4874 ? 4875 : 4874, 65536, NaN, "4874", "4875", null, undefined]) {
      expect(() => requireCLIPort(value, expected)).toThrow();
    }
    for (const value of ["", "0", "4864", String(expected === 4874 ? 4875 : 4874), "garbage"]) {
      expect(() => parseCLIPort(value, expected)).toThrow();
    }
  }
});

test("browser query and saved preferences fail closed independently without fallback", () => {
  expect(browserCLIPort(4875, new URLSearchParams(), 4875)).toBe(4875);
  expect(browserCLIPort(4875, new URLSearchParams("port=4875"), 4875)).toBe(4875);
  expect(browserCLIPort(4875, new URLSearchParams("port=4875&port=4875"), 4875)).toBe(4875);
  for (const query of ["port=4864", "port=4874", "port=", "port=garbage", "port=0", "port=4864&port=4875"]) {
    expect(() => browserCLIPort(4875, new URLSearchParams(query), 4875)).toThrow();
  }
  for (const saved of [4864, 4874, 0, "4875", undefined]) {
    expect(() => browserCLIPort(saved, new URLSearchParams("port=4875"), 4875)).toThrow();
  }
});
