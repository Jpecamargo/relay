fn main() {
    // generate_context! exige que o frontendDist (o build do front de apps/mac) exista; num checkout
    // limpo ainda não houve `pnpm build`.
    std::fs::create_dir_all("../../mac/dist").expect("criar apps/mac/dist");
    tauri_build::build();
}
