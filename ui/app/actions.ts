import { commandForFunctionKey } from "../commands";
import type { Action } from "../keys/keymap";
import { sendViewOp } from "../views/context";
import { runCommand } from "./dispatch";
import type { Workspace } from "./workspace";

/** Applies a keymap action to the workspace. */
export function runAction(action: Action, ws: Workspace): void {
  switch (action.type) {
    case "layout":
      ws.setLayout(action.layout);
      return;
    case "focus":
      ws.cycleFocus(action.delta);
      return;
    case "back":
      ws.back();
      return;
    case "help":
      runCommand("help", ws);
      return;
    case "view":
      sendViewOp(ws.state.focused, action.op);
      return;
    case "fkey": {
      const command = commandForFunctionKey(action.n);
      if (command) runCommand(command.id, ws);
      return;
    }
  }
}
