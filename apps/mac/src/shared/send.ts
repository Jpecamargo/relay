import { invokeCommand, type RemoteCommand } from "@relay/protocol-types";

/** Envia um comando remoto ao PC (`send_command`). */
export const send = (command: RemoteCommand) => invokeCommand("send_command", { command });
