/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.01 (2012-07)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

/*
 *------------------------------------------------------------------------
 *  File: qmfilt.c
 *  Function: Quadrature mirror filter (QMF)
 *            for band splitting and band reconstructing
 *------------------------------------------------------------------------
 */

#include "pcmswb_common.h"
#include "qmfilt.h"

#define LFRAME_HALF        (L_FRAME_SWB/2)
#define NTAP_QMF_HALF_MAX  (NTAP_QMF_SWB/2)
#define QMFBUFSIZE_MAX     (NTAP_QMF_SWB-2)

typedef struct {
  Short  ntap;
  Float  *f_bufmem;
  const Float *fQmf0;
  const Float *fQmf1;
} QMFilt_WORK;

/* Constructor */
void* QMFilt_const(int ntap, const Float *qmf0, const Float *qmf1)  /* returns pointer to work space */
{
  QMFilt_WORK *work = NULL;
  int qmfbufsize = ntap - 2;
  work = (QMFilt_WORK *)malloc(sizeof(QMFilt_WORK));
  if (work != NULL)
  {
    work->ntap = (Short)ntap;

    work->fQmf0 = qmf0;
    work->fQmf1 = qmf1;

    work->f_bufmem = (Float *)malloc(qmfbufsize * sizeof(Float));
    if (work->f_bufmem != NULL)
    {
      QMFilt_reset((void *)work);
    }
  }
  return (void *)work;
}

/* Destructor */
void  QMFilt_dest(void *ptr)
{
  QMFilt_WORK *work = (QMFilt_WORK *)ptr;
  if (work != NULL)
  {
    if (work->f_bufmem != NULL)
    {
      free( work->f_bufmem );
    }
    free( work );
  }
  return;
}

/* Reset */
void  QMFilt_reset(void *ptr)
{
  QMFilt_WORK *work = (QMFilt_WORK *)ptr;
  if (work != NULL)
  {
    zeroF(work->ntap-2, work->f_bufmem);
  }
}

/* Band splitting */
void  QMFilt_ana(
  int   n,       /* (i): Number of input signal      */
  Float *insig,  /* (i): Input signal                */
  Float *lsig,   /* (o): Output lower-band signal    */
  Float *hsig,   /* (o): Output higher-band signal   */
  void  *ptr     /* (i/o): Work space                */
)
{
  QMFilt_WORK *work = (QMFilt_WORK *)ptr;

  Short  i, j;
  Short  qmfbufsize = work->ntap - 2;
  Short  ntap_qmf_half = work->ntap / 2;

  Float  *f_insigpt;
  Float  f_insigbf[L_FRAME_SWB+QMFBUFSIZE_MAX];
  Float  fAcc0;
  Float  fAcc1;

  const Float *fQmf0 = work->fQmf0;
  const Float *fQmf1 = work->fQmf1;

  f_insigpt = f_insigbf;

  movF(qmfbufsize, work->f_bufmem, f_insigpt);
  f_insigpt = f_insigpt + qmfbufsize;
  movF(n, insig, f_insigpt);
  f_insigpt = f_insigpt + n;

  for (i=0 ; i<n ; i+=2)
  {
    f_insigpt = f_insigbf + i;

    fAcc0 = 0;
    fAcc1 = 0;

    for (j=0 ; j<ntap_qmf_half ; j++)
    {
      fAcc0 = fAcc0 + (*(f_insigpt++) * fQmf0[j]);
      fAcc1 = fAcc1 + (*(f_insigpt++) * fQmf1[j]);
    }
    *(lsig++) = (Float)roundFto16(fAcc0 + fAcc1);
    *(hsig++) = (Float)roundFto16(fAcc1 - fAcc0);
  }
  f_insigpt = f_insigbf + n;
  movF(qmfbufsize, f_insigpt, work->f_bufmem);
  return;
}

/* Band reconstructing */
void QMFilt_syn(
  int   n,       /* (i): Number of input signal       */
  Float *lsig,   /* (i): Input lower-band signal      */
  Float *hsig,   /* (i): Input higher-band signal     */
  Float *outsig, /* (o): Output 16-kHz-sampled signal */
  void  *ptr     /* (i/o): Pointer to work space      */
)
{
  QMFilt_WORK *work = (QMFilt_WORK *)ptr;

  Short  i;
  Float  fAcc0;
  Float  fAcc1;
  Float  *pf_buf;
  Float  *pf_sum;
  Float  *pf_dif;
  Float  buf_fsum[LFRAME_HALF+NTAP_QMF_HALF_MAX-1];
  Float  buf_fdif[LFRAME_HALF+NTAP_QMF_HALF_MAX-1];
  Short  ntap_qmf_half = work->ntap / 2;

  const Float *fQmf0 = work->fQmf0;
  const Float *fQmf1 = work->fQmf1;

  /* Overflow check */
  pf_buf = work->f_bufmem;
  pf_sum = buf_fsum;  
  pf_dif = buf_fdif;  

  /* copy from filter buffer */
  movF_ext((ntap_qmf_half-1), pf_buf, 2, pf_sum, 1);
  movF_ext((ntap_qmf_half-1), pf_buf+1, 2, pf_dif, 1);

  /* calculate sum/diff values */
  pf_sum = buf_fsum+ntap_qmf_half-1;
  pf_dif = buf_fdif+ntap_qmf_half-1;
  

  for (i=0 ; i<n ; i++)
  {
    *(pf_sum++) = lsig[i] + hsig[i];
    *(pf_dif++) = lsig[i] - hsig[i];
  }

  pf_sum = buf_fsum;
  pf_dif = buf_fdif;

  for (i=0 ; i<n ; i++)
  {
    fAcc0 = mac0_Array_f(ntap_qmf_half, (Float*)fQmf0, pf_sum);
    fAcc1 = mac0_Array_f(ntap_qmf_half, (Float*)fQmf1, pf_dif);

    *(outsig++) = (Float)roundFto16(fAcc1 * 2.0f);
    *(outsig++) = (Float)roundFto16(fAcc0 * 2.0f);

    pf_sum++;
    pf_dif++;
  }
  pf_buf = work->f_bufmem;

  movF_ext((ntap_qmf_half-1), pf_sum, 1, pf_buf, 2);
  movF_ext((ntap_qmf_half-1), pf_dif, 1, pf_buf+1, 2);

  return;
}
