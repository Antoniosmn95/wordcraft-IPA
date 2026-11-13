#ifndef WORDCRAFT_IOS_BRIDGING_HEADER_H
#define WORDCRAFT_IOS_BRIDGING_HEADER_H

#include <stdint.h>

/* Starts the Rust/egui app. `open_picker` asks the Swift host to show the system document
   picker: 0 documents, 1 pictures, 2 fonts, 3 recipient data, 4 Word sources. */
int32_t wc_ios_main(void (*open_picker)(uint32_t purpose));

#endif
