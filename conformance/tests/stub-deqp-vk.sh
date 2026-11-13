#!/bin/sh
# Stands in for deqp-vk in tests/deqp.rs: runs the cases of its case list by
# the last part of their names and prints what deqp-vk prints. Files in
# $STUB_STATE carry what one process does to the others.
#
#   *.pass     passes, and so does the runner's canary unless
#              $STUB_CANARY_FAILS is set
#   *.fail     fails
#   *.later    fails once $STUB_STATE/later.fails exists, and passes before
#   *.lose     loses the device: holds $STUB_STATE/lost for two seconds,
#              which the processes running at the time see, then ends the
#              run
#   *.victim   waits up to 1.5 s for $STUB_STATE/lost; loses the device when
#              it appears, and marks that in $STUB_STATE/victim.lost, and
#              passes otherwise
#   *.hurt     the same, but fails, and marks $STUB_STATE/hurt.failed
#   *.flip     loses the device the second time it runs, and passes every
#              other time
#   *.crowd    loses the device the first time it runs, and passes every
#              other time: a crash no other case explains
#   *.icb      passes with MESA_KK_DEBUG=force_icb, fails otherwise
#   *.hang     prints nothing more for 30 seconds
#   *.silent   ends the process at once, with nothing printed
#
# The probe's case prints the loader's driver messages for $VK_DRIVER_FILES
# and $STUB_DRIVER, and writes a test log with a driverVersion.
caselist=
qpa=
for arg in "$@"; do
    case "$arg" in
        --deqp-caselist-file=*) caselist=${arg#*=} ;;
        --deqp-log-filename=*) qpa=${arg#*=} ;;
    esac
done

# lose_device: print what deqp-vk prints for a lost device and end the run.
lose_device() {
    echo "  DeviceLost (VK_ERROR_DEVICE_LOST)"
    echo "Test run was ABORTED!"
    exit 1
}

# wait_lost: succeed once $STUB_STATE/lost exists, within 1.5 s.
wait_lost() {
    tries=0
    while [ "$tries" -lt 15 ]; do
        [ -e "$STUB_STATE/lost" ] && return 0
        sleep 0.1
        tries=$((tries + 1))
    done
    return 1
}

# count <name>: print how many times this case of <name> has run, this one
# included.
count() {
    n=$(($(cat "$STUB_STATE/$1.count" 2>/dev/null || echo 0) + 1))
    echo "$n" > "$STUB_STATE/$1.count"
    echo "$n"
}

while read -r case; do
    [ -n "$case" ] || continue
    case "$case" in
        *.silent) exit 1 ;;
    esac
    printf "\nTest case '%s'..\n" "$case"
    case "$case" in
        dEQP-VK.info.device_properties)
            echo "[Vulkan Loader] DRIVER:         Found ICD manifest file $VK_DRIVER_FILES/stub_icd.json, version 1.0.1"
            echo "[Vulkan Loader] DRIVER:                Using \"Stub GPU\" with driver: \"$STUB_DRIVER\""
            printf '\tdriverVersion = 109457507\n' > "$qpa"
            echo "  Pass (Pass)"
            ;;
        dEQP-VK.api.smoke.triangle)
            if [ -n "$STUB_CANARY_FAILS" ]; then
                echo "  Fail (no triangle)"
            else
                echo "  Pass (Rendering succeeded)"
            fi
            ;;
        *.pass)
            echo "  Pass (Pass)"
            ;;
        *.fail)
            echo "  Fail (bad result)"
            ;;
        *.later)
            if [ -e "$STUB_STATE/later.fails" ]; then
                echo "  Fail (bad result)"
            else
                echo "  Pass (Pass)"
            fi
            ;;
        *.lose)
            touch "$STUB_STATE/lost"
            sleep 2
            rm -f "$STUB_STATE/lost"
            lose_device
            ;;
        *.victim)
            if wait_lost; then
                touch "$STUB_STATE/victim.lost"
                lose_device
            fi
            echo "  Pass (Pass)"
            ;;
        *.hurt)
            if wait_lost; then
                touch "$STUB_STATE/hurt.failed"
                echo "  Fail (wrong result)"
            else
                echo "  Pass (Pass)"
            fi
            ;;
        *.flip)
            [ "$(count flip)" -eq 2 ] && lose_device
            echo "  Pass (Pass)"
            ;;
        *.crowd)
            [ "$(count crowd)" -eq 1 ] && lose_device
            echo "  Pass (Pass)"
            ;;
        *.icb)
            if [ "$MESA_KK_DEBUG" = force_icb ]; then
                echo "  Pass (Pass)"
            else
                echo "  Fail (no MESA_KK_DEBUG=force_icb)"
            fi
            ;;
        *.hang)
            sleep 30
            ;;
    esac
done < "$caselist"
echo "DONE!"
