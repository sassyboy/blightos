#!/bin/bash

# Define variables
IMAGE_NAME="build/disk.img"
IMAGE_SIZE="500M" # Size of the disk image
SOURCE_DIR="kernel" # Directory to copy files from
MOUNT_POINT="/mnt/img_mount"

# --- 1. Create an empty disk image file ---
echo "Creating empty disk image file: $IMAGE_NAME (${IMAGE_SIZE})..."
dd if=/dev/zero of="$IMAGE_NAME" bs=1M count=$(echo "$IMAGE_SIZE" | sed 's/M//') status=progress
if [ $? -ne 0 ]; then echo "Error: dd failed"; exit 1; fi

# --- 2. Create a GPT partition table ---
echo "Creating GPT partition table..."
parted --script "$IMAGE_NAME" mktable gpt
if [ $? -ne 0 ]; then echo "Error: parted mktable failed"; exit 1; fi

# --- 3. Create a single FAT32 partition filling the whole disk ---
echo "Creating FAT32 partition..."
parted --script "$IMAGE_NAME" mkpart primary fat32 1MiB 100%
if [ $? -ne 0 ]; then echo "Error: parted mkpart failed"; exit 1; fi

# --- 4. Set up a loop device and make the partition visible ---
echo "Setting up loop device and making partitions visible..."
LOOP_DEV=$(sudo losetup -f --show "$IMAGE_NAME")
if [ $? -ne 0 ]; then echo "Error: losetup failed"; exit 1;
fi

# Inform user to wait for partition discovery (partprobe or kpartx)
echo "Loop device created: $LOOP_DEV"
sleep 2 # Give the kernel a moment to recognize the new partition

# Use kpartx to create device maps for partitions if losetup -P isn't available
# If using a modern losetup with the -P flag in step 4, this might not be needed.
# Let's use partprobe on the loop device to ensure partitions are found.
sudo partprobe "$LOOP_DEV"
if [ $? -ne 0 ]; then
    echo "partprobe failed, attempting with kpartx..."
    kpartx -a "$LOOP_DEV"
    # Find the mapped partition device (e.g., /dev/mapper/loop0p1)
    PART_DEV="/dev/mapper/$(basename "$LOOP_DEV")p1"
else
    # Assuming losetup -P or partprobe works, the partition device name will be like /dev/loop0p1
    PART_DEV="${LOOP_DEV}p1"
fi

# Ensure the partition device exists
if [ ! -b "$PART_DEV" ]; then
    echo "Error: Partition device $PART_DEV not found. Check kpartx or losetup -P functionality."
    sudo losetup -d "$LOOP_DEV"
    exit 1
fi

# --- 5. Format the partition as FAT32 ---
echo "Formatting partition $PART_DEV as FAT32..."
sudo mkfs.fat -F 32 "$PART_DEV" -n MYDISK
if [ $? -ne 0 ]; then echo "Error: mkfs.fat failed"; cleanup; exit 1; fi

# --- 6. Mount the partition ---
echo "Mounting partition to $MOUNT_POINT..."
sudo mkdir -p "$MOUNT_POINT"
sudo mount "$PART_DEV" "$MOUNT_POINT"
if [ $? -ne 0 ]; then echo "Error: mount failed"; cleanup; exit 1; fi

# --- 7. Copy contents of the source directory ---
echo "Copying contents of $SOURCE_DIR into the image..."
if [ -d "$SOURCE_DIR" ]; then
    sudo cp -r "$SOURCE_DIR"/* "$MOUNT_POINT"/
    sudo mkdir "$MOUNT_POINT"/blightos
    sudo mkdir "$MOUNT_POINT"/blightos/res
    sudo cp -r build/kernel.elf "$MOUNT_POINT"/blightos/
    sudo cp -r build/*.box "$MOUNT_POINT"/blightos/
    sudo cp -r resources/* "$MOUNT_POINT"/blightos/res/
    if [ $? -ne 0 ]; then echo "Error: cp failed"; cleanup; exit 1; fi
else
    echo "Warning: Source directory $SOURCE_DIR does not exist. Skipping file copy."
fi

# --- 8. Cleanup: Unmount and detach loop device ---
echo "Cleaning up..."
sudo umount "$MOUNT_POINT"
if [ $? -ne 0 ]; then echo "Error: umount failed"; exit 1; fi
sudo rmdir "$MOUNT_POINT"

if command -v kpartx &> /dev/null && [ -n "$PART_DEV" ] && [[ "$PART_DEV" == *"/dev/mapper"* ]]; then
    kpartx -d "$LOOP_DEV"
fi

sudo losetup -d "$LOOP_DEV"
if [ $? -ne 0 ]; then echo "Error: losetup -d failed"; exit 1; fi

echo "Disk image $IMAGE_NAME created, formatted, and files copied successfully."
