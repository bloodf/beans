import { ScrollView } from "react-native";
import { DeviceRows } from "../../../../src/ui/DeviceRows";

export default function DevicesScreen() {
  return (
    <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
      <DeviceRows base="/devices" />
    </ScrollView>
  );
}
