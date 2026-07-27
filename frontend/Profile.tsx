import React from "react";
import styles from "./Profile.module.css";

interface ProfileData {
  address: string;
  displayName: string;
  joinedDate: string;
  totalStreamsCreated: number;
  totalStreamsReceived: number;
}

interface ProfileProps {
  profile?: ProfileData;
}

export const Profile: React.FC<ProfileProps> = ({ profile: profileProp }) => {
  const profile = profileProp ?? {
    address: "",
    displayName: "Anonymous",
    joinedDate: "",
    totalStreamsCreated: 0,
    totalStreamsReceived: 0,
  };

  return (
    <div className={styles.profile}>
      <h2 className={styles.title}>Profile</h2>
      
      <div className={styles.avatarSection}>
        <div className={styles.avatar}>
          {profile.displayName?.charAt(0).toUpperCase() || "A"}
        </div>
        <div className={styles.userInfo}>
          <p className={styles.displayName}>{profile.displayName}</p>
          <p className={styles.address}>{profile.address}</p>
        </div>
      </div>

      <div className={styles.stats}>
        <div className={styles.statItem}>
          <span className={styles.statLabel}>Joined</span>
          <span className={styles.statValue}>{profile.joinedDate || "N/A"}</span>
        </div>
        <div className={styles.statItem}>
          <span className={styles.statLabel}>Streams Created</span>
          <span className={styles.statValue}>{profile.totalStreamsCreated}</span>
        </div>
        <div className={styles.statItem}>
          <span className={styles.statLabel}>Streams Received</span>
          <span className={styles.statValue}>{profile.totalStreamsReceived}</span>
        </div>
      </div>
    </div>
  );
};
