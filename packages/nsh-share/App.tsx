import { mount, signal, Text, View } from "nsh";
import { memoryBackend, Share, ShareError } from "./src";

/** The demo runs on a memory backend, so the entry never calls the native module; the press
 * delivers the sheet's answer itself, so the one operation always resolves. */
const backend = memoryBackend();
const share = new Share({ backend });

const message = signal("nothing yet");

function codeOf(error: unknown): string {
  return error instanceof ShareError ? error.code : "unknown error";
}

function shareText() {
  message.set("sharing");
  share.share({ text: "Shared from nsh-share" }).then(
    (result) => message.set(`share ${result.status}`),
    (error: unknown) => message.set(codeOf(error)),
  );
  // No system answers a memory backend, so the press plays the system: a dismissal, once.
  backend.deliverShare({ raw: "", status: "dismissed" });
}

function App() {
  return (
    <View class="flex-1 gap-3 p-4">
      <Text testID="title" class="text-foreground">nsh-share</Text>
      <Text testID="message" class="text-foreground">{message()}</Text>
      <Text testID="share" role="button" onPress={shareText} class="rounded-(--radius) bg-muted px-3 py-2">
        Share text
      </Text>
    </View>
  );
}

mount(App);
