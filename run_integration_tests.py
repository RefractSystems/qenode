import subprocess
import os

tests_dir = "tests/native_integration/tests"
test_files = [f for f in os.listdir(tests_dir) if f.endswith(".rs")]
test_names = [os.path.splitext(f)[0] for f in test_files]

results = []

for test_name in sorted(test_names):
    print(f"Running test: {test_name}...")
    try:
        # Using --release as in the Makefile, and nightly with bindeps
        process = subprocess.run(
            ["cargo", "+nightly", "test", "-Z", "bindeps", "-p", "native-integration", "--test", test_name, "--release"],
            capture_output=True,
            text=True
        )
        if process.returncode == 0:
            results.append((test_name, "PASS"))
            print(f"RESULT: {test_name} PASS")
        else:
            results.append((test_name, "FAIL"))
            print(f"RESULT: {test_name} FAIL")
            print(f"STDOUT: {process.stdout}")
            print(f"STDERR: {process.stderr}")
    except Exception as e:
        results.append((test_name, f"ERROR: {str(e)}"))
        print(f"RESULT: {test_name} ERROR")

print("\n--- Summary ---")
for test_name, result in results:
    print(f"{test_name}: {result}")
