#include "tessipher.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include <stdalign.h>
static uint8_t core_scratch[8192], detector_scratch[8192], payload[7089];
static qr_segment segments[16];
static uint8_t matrix[56], pixels[116*116];
int main(void) {
    qr_config config; qr_memory_info memory; qr_result result;
    assert(qr_default_config(NULL)==1);
    assert(qr_default_config(&config)==0);
    assert(qr_memory(&config,&memory)==0);
    assert(memory.abi_version==1 && memory.max_width>=116 && memory.max_height>=116);
    assert(memory.core_scratch_bytes<=sizeof core_scratch);
    assert(memory.detector_scratch_bytes<=sizeof detector_scratch);
    assert(memory.segment_size==sizeof(qr_segment));
    assert(memory.segment_alignment==alignof(qr_segment));
    assert(memory.result_size==sizeof(qr_result));
    assert(memory.result_alignment==alignof(qr_result));
    FILE *f=fopen("fixtures/v1.bin","rb");assert(f);assert(fread(matrix,1,sizeof matrix,f)==sizeof matrix);fclose(f);
    assert(qr_decode_matrix(matrix,sizeof matrix,21,core_scratch,sizeof core_scratch,payload,sizeof payload,segments,16,&result)==0);
    assert(result.version==1 && result.ec_level==3 && result.mask==3 && result.payload_len==4);
    assert(memcmp(payload,"TEST",4)==0 && segments[0].mode==4 && segments[0].eci==3);
    assert(qr_decode_matrix(NULL,1,21,core_scratch,sizeof core_scratch,payload,sizeof payload,segments,16,&result)==1);
    assert(qr_decode_matrix(matrix,sizeof matrix,21,core_scratch,0,payload,sizeof payload,segments,16,&result)==102);
    assert(qr_decode_matrix(matrix,sizeof matrix,21,core_scratch,sizeof core_scratch,payload,3,segments,16,&result)==109);
    assert(qr_decode_matrix(matrix,sizeof matrix,21,core_scratch,sizeof core_scratch,payload,sizeof payload,NULL,0,&result)==110);
    assert(qr_decode_matrix(matrix,sizeof matrix,21,core_scratch,sizeof core_scratch,payload,sizeof payload,(qr_segment*)((uint8_t*)segments+1),1,&result)==1);
    assert(qr_decode_matrix(matrix,sizeof matrix,21,matrix,sizeof matrix,payload,sizeof payload,segments,16,&result)==2);
    assert(qr_decode_matrix(matrix,SIZE_MAX,21,core_scratch,sizeof core_scratch,payload,sizeof payload,segments,16,&result)==1);
    memset(pixels,255,sizeof pixels);
    for(size_t y=0;y<21;y++)for(size_t x=0;x<21;x++)if(matrix[(y*21+x)/8]&(128>>((y*21+x)%8)))for(size_t j=0;j<4;j++)for(size_t i=0;i<4;i++)pixels[((y+4)*4+j)*116+(x+4)*4+i]=0;
    assert(qr_decode_gray(&config,pixels,sizeof pixels,116,116,116,detector_scratch,sizeof detector_scratch,core_scratch,sizeof core_scratch,payload,sizeof payload,segments,16,&result)==0);
    assert(result.payload_len==4 && memcmp(payload,"TEST",4)==0);
    assert(qr_decode_gray(&config,pixels,sizeof pixels,116,116,115,detector_scratch,sizeof detector_scratch,core_scratch,sizeof core_scratch,payload,sizeof payload,segments,16,&result)==10);
    printf("C ABI passed: core=%zu detector=%zu alignment=%zu\n",memory.core_scratch_bytes,memory.detector_scratch_bytes,memory.scratch_alignment);
}
