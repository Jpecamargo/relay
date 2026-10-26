// @vitest-environment jsdom
import type { LayoutProfile } from "@relay/protocol-types";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { Editor } from "./index";

// jsdom não repassa clientX/clientY nos PointerEvent: usa MouseEvent com pointerId.
class TestPointerEvent extends MouseEvent {
  pointerId: number;
  constructor(type: string, init: MouseEventInit & { pointerId?: number } = {}) {
    super(type, init);
    this.pointerId = init.pointerId ?? 1;
  }
}
vi.stubGlobal("PointerEvent", TestPointerEvent);

const mock = vi.hoisted(() => ({ invoke: vi.fn(), handlers: new Map<string, (p: unknown) => void>() }));
vi.mock("@relay/protocol-types", () => ({
  invokeCommand: mock.invoke,
  listenEvent: async (name: string, cb: (p: unknown) => void) => {
    mock.handlers.set(name, cb);
    return () => mock.handlers.delete(name);
  },
}));

const info = (id: string, name: string, machine: string, mm: [number, number]) => ({
  id, name, machine, origin_px: [0, 0], size_px: [1000, 1000], scale: 1, size_mm: mm, is_primary: true, is_builtin: false,
});
const placed = (id: string, machine: string, x: number, y: number, w: number, h: number) => ({
  display_id: id, machine, rect_mm: { x, y, w, h }, sensitivity: 1,
});
const profile: LayoutProfile = {
  key: ["mac", "d1", "d2"],
  name: "Mesa",
  displays: [placed("mac", "Mac", 0, 140, 302, 196), placed("d1", "Windows", 302, 0, 597, 336), placed("d2", "Windows", 899, 0, 597, 336)],
  windows_detached: false,
} as LayoutProfile;

let status: Record<string, unknown> = {};

function backend(warnings: unknown[] = []) {
  mock.invoke.mockImplementation(async (name: string) => {
    if (name === "get_displays")
      return { local: [info("mac", "MacBook Pro", "Mac", [302, 196])], peer: [info("d1", "Dell 1", "Windows", [597, 336]), info("d2", "Dell 2", "Windows", [597, 336])] };
    if (name === "get_layout") return profile;
    if (name === "get_layout_book") return { revision: 1, host: "Mac", profiles: [], active: null };
    if (name === "get_status") return { peer_name: "Dell-PC", peer_version: "0.3.0", layout_host: "Local", layout_editable: true, ...status };
    if (name === "save_layout") return { warnings };
  });
}

afterEach(() => {
  cleanup();
  mock.invoke.mockReset();
  mock.handlers.clear();
  status = {};
});

const rectOf = (id: string) => screen.getByTestId(`display-${id}`).querySelector("rect")!;

test("arrastar um Dell move o bloco, destaca a borda e salvar envia o perfil com warnings como Banner", async () => {
  backend([{ kind: "DockAutohideEdge", message: "Dock na borda direita", display_id: null }]);
  render(<Editor />);
  await screen.findByTestId("display-mac");

  // Mac está em y=140, Dell em y=0: segmento inicial = y em [140, 336] ∩ [0,336] -> [140, 196+140]
  expect(screen.getByTestId("shared-edge").getAttribute("data-segment")).toBe("140.0,336.0");

  const dell = screen.getByTestId("display-d1");
  fireEvent.pointerDown(dell, { clientX: 100, clientY: 100, pointerId: 1 });
  fireEvent.pointerMove(dell, { clientX: 102, clientY: 37, pointerId: 1 }); // dy = -63 -> y=-63 ; snap topo do Mac (140) não, mas 5mm
  fireEvent.pointerMove(dell, { clientX: 102, clientY: 237, pointerId: 1 }); // dy = +137 -> y=137, snapa em 140
  fireEvent.pointerUp(dell, { pointerId: 1 });

  await waitFor(() => expect(rectOf("d1").getAttribute("y")).toBe("140"));
  expect(rectOf("d2").getAttribute("y")).toBe("140"); // bloco
  expect(rectOf("d2").getAttribute("x")).toBe("899"); // posição relativa preservada
  expect(screen.getByTestId("shared-edge").getAttribute("data-segment")).toBe("140.0,336.0");

  fireEvent.click(screen.getByRole("button", { name: "Salvar" }));
  await screen.findByText("Dock na borda direita");
  const call = mock.invoke.mock.calls.find((c) => c[0] === "save_layout")!;
  expect(call[1].profile.displays[1].rect_mm.y).toBe(140);
});

test("Mac não arrasta; alinhar pelo topo leva o bloco Windows ao topo do Mac", async () => {
  backend();
  render(<Editor />);
  await screen.findByTestId("display-mac");
  const macG = screen.getByTestId("display-mac");
  fireEvent.pointerDown(macG, { clientX: 0, clientY: 0, pointerId: 1 });
  fireEvent.pointerMove(macG, { clientX: 50, clientY: 50, pointerId: 1 });
  fireEvent.pointerUp(macG, { pointerId: 1 });
  expect(rectOf("mac").getAttribute("y")).toBe("140");

  fireEvent.click(screen.getByRole("button", { name: "Alinhar pelo topo" }));
  expect(rectOf("d1").getAttribute("y")).toBe("140");
  expect(screen.getByTestId("shared-edge").getAttribute("data-segment")).toBe("140.0,336.0");
});

test("painel lateral mostra o pulo (Ctrl+Option+n) e sensibilidade 0,25 a 4,0 por tela", async () => {
  backend();
  render(<Editor />);
  await screen.findByTestId("display-mac");
  expect(screen.getByLabelText("Ctrl Option 3")).toBeTruthy();
  const sliders = screen.getAllByRole("slider") as HTMLInputElement[];
  expect(sliders).toHaveLength(3);
  expect([sliders[0]!.min, sliders[0]!.max]).toEqual(["0.25", "4"]);
  fireEvent.change(sliders[1]!, { target: { value: "2" } });
  fireEvent.click(screen.getByRole("button", { name: "Salvar" }));
  await waitFor(() => expect(mock.invoke.mock.calls.some((c) => c[0] === "save_layout")).toBe(true));
  const saved = mock.invoke.mock.calls.find((c) => c[0] === "save_layout")![1].profile;
  expect(saved.displays[1].sensitivity).toBe(2);
});

test("orientação por tela: Retrato troca largura e altura do Dell e salva o override", async () => {
  backend();
  render(<Editor />);
  await screen.findByTestId("display-mac");
  const selects = screen.getAllByLabelText("Orientação") as HTMLSelectElement[];
  expect(selects).toHaveLength(2); // só as telas Windows
  fireEvent.change(selects[0]!, { target: { value: "90" } });
  // o Dell 1 é 597x336 no fixture e o mesmo EDID em retrato vira 336x597
  expect(screen.getByTestId("display-d1").textContent).toContain("336 × 597 mm");
  fireEvent.click(screen.getByRole("button", { name: "Salvar" }));
  await waitFor(() => expect(mock.invoke.mock.calls.some((c) => c[0] === "save_layout")).toBe(true));
  const saved = mock.invoke.mock.calls.find((c) => c[0] === "save_layout")![1].profile;
  expect(saved.displays[1].rotation).toBe(90);
});

test("novo perfil copia a disposição atual com nome livre e passa a ser o perfil em uso", async () => {
  backend();
  render(<Editor />);
  await screen.findByTestId("display-mac");
  expect(screen.getByRole("button", { name: "Excluir perfil" })).toHaveProperty("disabled", true);

  fireEvent.click(screen.getByRole("button", { name: "Novo perfil" }));
  await waitFor(() => expect(mock.invoke.mock.calls.some((c) => c[0] === "set_layout_profiles")).toBe(true));
  const [, args] = mock.invoke.mock.calls.find((c) => c[0] === "set_layout_profiles")!;
  expect(args.profiles.map((p: LayoutProfile) => p.name)).toEqual(["Mesa", "Perfil 2"]);
  expect(args.active).toBe("Perfil 2");
  expect(args.profiles[1].displays).toEqual(profile.displays);
});

test("salvar com outro nome renomeia o perfil em vez de criar outro", async () => {
  backend();
  render(<Editor />);
  await screen.findByTestId("display-mac");
  fireEvent.change(screen.getByLabelText("Nome do perfil"), { target: { value: "Escritório" } });
  fireEvent.click(screen.getByRole("button", { name: "Salvar" }));
  await waitFor(() => expect(mock.invoke.mock.calls.some((c) => c[0] === "save_layout")).toBe(true));
  const rename = mock.invoke.mock.calls.find((c) => c[0] === "set_layout_profiles")!;
  expect(rename[1]).toEqual({ profiles: [{ ...profile, name: "Escritório" }], active: "Escritório" });
  expect(mock.invoke.mock.calls.find((c) => c[0] === "save_layout")![1].profile.name).toBe("Escritório");
});

test("editável (Mac): selo Este computador na tela local e sem aviso", async () => {
  backend();
  render(<Editor />);
  await screen.findByTestId("display-mac");
  expect(screen.getByTestId("display-mac").textContent).toContain("Este computador");
  expect(screen.getByTestId("display-d1").textContent).not.toContain("Este computador");
  expect(screen.queryByText(/Os layouts ficam em/)).toBeNull();
  expect(screen.getByRole("button", { name: "Salvar" })).toBeTruthy();
});

async function readOnly(extra: Record<string, unknown>) {
  status = { layout_host: "Peer", layout_editable: false, ...extra };
  backend();
  render(<Editor />);
  await screen.findByTestId("display-mac");
  await screen.findByText(/Dell-PC/);
}

test("só-leitura com host offline: avisa, desabilita controles e não arrasta", async () => {
  await readOnly({ peer_version: null });
  expect(screen.getByText(/Os layouts ficam em Dell-PC. Conecte-o/)).toBeTruthy();
  for (const n of ["Novo perfil", "Excluir perfil", "Alinhar pelo topo", "Salvar"])
    expect(screen.getByRole("button", { name: n })).toHaveProperty("disabled", true);
  expect(screen.getByLabelText("Nome do perfil")).toHaveProperty("disabled", true);
  for (const sl of screen.getAllByRole("slider")) expect(sl).toHaveProperty("disabled", true);
  const dell = screen.getByTestId("display-d1");
  fireEvent.pointerDown(dell, { clientX: 100, clientY: 100, pointerId: 1 });
  fireEvent.pointerMove(dell, { clientX: 102, clientY: 237, pointerId: 1 });
  fireEvent.pointerUp(dell, { pointerId: 1 });
  expect(rectOf("d1").getAttribute("y")).toBe("0");
});

test("só-leitura com peer 0.2: pede para atualizar o Relay no peer", async () => {
  await readOnly({ peer_version: "0.2.4" });
  expect(screen.getByText(/Atualize o Relay em Dell-PC/)).toBeTruthy();
});

test("relay:layout recarrega o livro, mantém a tela selecionada e descarta o arraste", async () => {
  backend();
  render(<Editor />);
  await screen.findByTestId("display-mac");
  fireEvent.click(screen.getByRole("button", { name: /Dell 2/ }));
  const dell = screen.getByTestId("display-d1");
  fireEvent.pointerDown(dell, { clientX: 100, clientY: 100, pointerId: 1 });
  fireEvent.pointerMove(dell, { clientX: 102, clientY: 237, pointerId: 1 });
  expect(rectOf("d1").getAttribute("y")).not.toBe("0");
  const before = mock.invoke.mock.calls.filter((c) => c[0] === "get_layout_book").length;
  await waitFor(() => expect(mock.handlers.has("relay:layout")).toBe(true));
  act(() => mock.handlers.get("relay:layout")!({ revision: 2, host: "Mac", profiles: [], active: null }));
  await waitFor(() => expect(mock.invoke.mock.calls.filter((c) => c[0] === "get_layout_book").length).toBe(before + 1));
  await waitFor(() => expect(rectOf("d1").getAttribute("y")).toBe("0"));
  expect(screen.getByRole("button", { name: /Dell 1/ }).parentElement!.getAttribute("data-selected")).toBe("true");
});
