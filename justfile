id := "com.evilbug.infobarutils.sdPlugin"
# Elgato bundle id; the UUID must prefix the keypad action UUIDs.
elgato_id := "com.evilbug.infobarclock.sdPlugin"

release: bump package tag

package: build-linux collect zip

bump next=`git cliff --bumped-version | tr -d "v"`:
    git diff --cached --exit-code

    echo "We will bump version to {{next}}, press any key"
    read ans

    sed -i 's/"Version": ".*"/"Version": "{{next}}"/g' assets/manifest.json
    sed -i 's/^version = ".*"$/version = "{{next}}"/g' Cargo.toml

tag next=`git cliff --bumped-version`:
    echo "Generating changelog"
    git cliff -o CHANGELOG.md --tag {{next}}

    echo "We will now commit the changes, please review before pressing any key"
    read ans

    git add .
    git commit -m "chore(release): {{next}}"
    git tag "{{next}}"

build-linux:
    cargo build --release --target x86_64-unknown-linux-gnu --target-dir target/plugin-linux

build-mac:
    docker run --rm -v $(pwd):/workspace -w /workspace ghcr.io/rust-cross/cargo-zigbuild:latest cargo zigbuild --release --target universal2-apple-darwin --target-dir target/plugin-mac

build-win:
    cargo build --release --target x86_64-pc-windows-gnu --target-dir target/plugin-win

package-all: build-linux build-mac build-win collect-all zip

# Build the keypad-only bundle for the official Elgato Stream Deck app (Windows).
package-elgato: build-win collect-elgato

collect-elgato:
    rm -rf build-elgato
    mkdir -p build-elgato/{{elgato_id}}/icons build-elgato/{{elgato_id}}/propertyInspector
    cp assets/manifest.elgato.json build-elgato/{{elgato_id}}/manifest.json
    cp assets/icons/claude.png build-elgato/{{elgato_id}}/icons/
    cp assets/propertyInspector/claude-key.html build-elgato/{{elgato_id}}/propertyInspector/
    cp assets/propertyInspector/sdpi.css build-elgato/{{elgato_id}}/propertyInspector/
    cp target/plugin-win/x86_64-pc-windows-gnu/release/opendeck-infobar-utils.exe build-elgato/{{elgato_id}}/opendeck-infobar-utils.exe

collect:
    rm -rf build
    mkdir -p build/{{id}}
    cp assets/manifest.json build/{{id}}/
    cp -r assets/icons build/{{id}}/
    cp -r assets/propertyInspector build/{{id}}/
    cp target/plugin-linux/x86_64-unknown-linux-gnu/release/opendeck-infobar-utils build/{{id}}/opendeck-infobar-utils-linux

collect-all:
    rm -rf build
    mkdir -p build/{{id}}
    cp assets/manifest.json build/{{id}}/
    cp -r assets/icons build/{{id}}/
    cp -r assets/propertyInspector build/{{id}}/
    cp target/plugin-linux/x86_64-unknown-linux-gnu/release/opendeck-infobar-utils build/{{id}}/opendeck-infobar-utils-linux
    cp target/plugin-mac/universal2-apple-darwin/release/opendeck-infobar-utils build/{{id}}/opendeck-infobar-utils-mac
    cp target/plugin-win/x86_64-pc-windows-gnu/release/opendeck-infobar-utils.exe build/{{id}}/opendeck-infobar-utils-win.exe

[working-directory: "build"]
zip:
    zip -r opendeck-infobar-utils.plugin.zip {{id}}/
