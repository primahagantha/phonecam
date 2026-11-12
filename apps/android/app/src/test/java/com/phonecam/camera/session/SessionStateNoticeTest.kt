package com.phonecam.camera.session

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The rule that a failure cannot sit beside a notice.
 *
 * A notice describes the moment it was posted, and an error is a later, different
 * fact. The phone was seen showing both at once: a stale "approve the connection"
 * instruction left over from the pairing attempt, under the error that the
 * approval never came. [SessionState.failed] is the single transition both
 * failure paths go through, so the rule is testable without a live session.
 */
class SessionStateNoticeTest {

    private val approvalNotice =
        SessionState(
            phase = SessionState.Phase.CONNECTING,
            notice = "Check your PC: approve the connection request from this phone.",
            noticeKind = NoticeKind.Approval,
        )

    @Test
    fun `a fresh state has no notice and the default kind`() {
        assertEquals(NoticeKind.Substitution, SessionState().noticeKind)
        assertNull(SessionState().notice)
    }

    @Test
    fun `a failure clears the notice it superseded`() {
        val failed = approvalNotice.failed("PhoneCam couldn't reach your PC.")

        assertEquals(SessionState.Phase.ERROR, failed.phase)
        assertEquals("PhoneCam couldn't reach your PC.", failed.error)
        assertNull(failed.notice)
    }

    @Test
    fun `a failure resets the notice kind so a later substitution is not titled as an approval`() {
        val failed = approvalNotice.failed("PhoneCam couldn't reach your PC.")

        assertEquals(NoticeKind.Substitution, failed.noticeKind)
    }

    @Test
    fun `a failure with nothing to clear still fails`() {
        val failed = SessionState().failed("The PC declined the connection.")

        assertEquals(SessionState.Phase.ERROR, failed.phase)
        assertEquals("The PC declined the connection.", failed.error)
        assertNull(failed.notice)
    }

    @Test
    fun `the approval notice is not a substitution`() {
        assertEquals(NoticeKind.Approval, approvalNotice.noticeKind)
    }
}
