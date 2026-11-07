import PhotosSource

@main
struct Checks {
    static func main() throws {
        #if LAB
        try PhotosChecks.run()
        print("all Photos checks passed")
        #endif
    }
}
