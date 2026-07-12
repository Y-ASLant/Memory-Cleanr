fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=App.ico");

    let mut res = winres::WindowsResource::new();
    res.set_icon("App.ico");
    res.compile().expect("failed to compile Windows resource");
}
